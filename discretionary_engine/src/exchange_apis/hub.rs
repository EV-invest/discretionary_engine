use std::{collections::HashMap, sync::Arc};

use ahash::AHashMap;
use color_eyre::eyre::Result;
use tokio::{
	select,
	sync::{mpsc, watch},
	task::JoinSet,
};
use tracing::{Span, debug, field::Empty, instrument};
use uuid::Uuid;

use super::exchanges::Exchanges;
use crate::{
	PositionOrderId,
	config::LiveSettings,
	exchange_apis::{
		Market, binance, order_types,
		order_types::{ConceptualOrder, ConceptualOrderType, Order, ProtocolOrderId},
	},
	positions::HubToPosition,
	protocols::{ProtocolFill, ProtocolFills},
};

#[derive(Clone, Debug, Default, derive_new::new)]
pub struct ExchangeToHub {
	pub key: Uuid,
	/// Market from which the fill comes
	pub market: Market,
	pub fill_qty: f64,
	pub order: Order<PositionOrderId>,
}

#[derive(Clone, Debug, Default, derive_new::new)]
pub struct HubToExchange {
	pub key: Uuid,
	pub orders: Vec<Order<PositionOrderId>>,
}

#[instrument(skip_all)]
pub fn init_hub(live_settings: Arc<LiveSettings>, parent_js: &mut JoinSet<Result<()>>, exchanges: Arc<Exchanges>) -> mpsc::Sender<PositionToHub> {
	let (tx, rx) = mpsc::channel(32);
	parent_js.spawn(hub(live_settings.clone(), rx, exchanges));
	tx
}

#[derive(Clone, Debug, derive_new::new)]
pub struct PositionToHub {
	key: Uuid,
	orders: Vec<ConceptualOrder<ProtocolOrderId>>,
	position_callback: HubToPosition,
}

#[instrument(skip_all)]
pub async fn hub(live_settings: Arc<LiveSettings>, mut rx: mpsc::Receiver<PositionToHub>, exchanges: Arc<Exchanges>) -> Result<()> {
	// TODO!!: assert all protocol orders here with trigger prices have them above/below current price in accordance to order's side.
	//- init the runtime of exchanges

	let (fills_tx, mut fills_rx) = mpsc::channel::<ExchangeToHub>(32);
	let (orders_tx, orders_rx) = watch::channel::<HubToExchange>(HubToExchange::default());
	let mut js = JoinSet::default();

	// Spawn Binance
	let exchanges_clone = exchanges.clone();
	let live_settings_clone = live_settings.clone();
	js.spawn(async move {
		let mut exchange_runtimes_js = JoinSet::default();
		binance::binance_runtime(live_settings_clone, &mut exchange_runtimes_js, fills_tx, orders_rx, exchanges_clone.binance.clone()).await;
		unreachable!();
		//exchange_runtimes_js.join_all().await;
	});

	let mut positions_local_knowledge: AHashMap<Uuid, PositionLocalKnowledge> = AHashMap::default();
	let mut exchanges_local_knowledge: AHashMap<Market, ExchangeLocalKnowledge> = AHashMap::default();

	//LOOP: Main hub loop, runs forever
	loop {
		select! {
			Some(update_from_position) = rx.recv() => {
				handle_update_from_position(update_from_position, &mut positions_local_knowledge, &orders_tx, &mut exchanges_local_knowledge)?;
			},
			Some(fill) = fills_rx.recv() => {
				let exchange_local_knowledge = exchanges_local_knowledge.entry(fill.market).or_default();
				exchange_local_knowledge.key = fill.key;
				//TODO!!!: update our knowledge of exchange's target_orders. Currently returning directly, which is a hack that would only work with one Market.
				let position_local_knowledge = positions_local_knowledge.get_mut(&fill.order.id.position_id).expect("Can't receive a fill without a position first requesting those orders");
				handle_fill(fill, position_local_knowledge).await?;
			},
			else => break,
		}
	}

	js.join_all().await;
	Ok(())
}
#[derive(Clone, Debug, derive_new::new)]
struct PositionLocalKnowledge {
	pub key: Uuid,
	pub callback: mpsc::Sender<ProtocolFills>,
	pub requested_orders: Vec<ConceptualOrder<ProtocolOrderId>>,
}

#[derive(Clone, Debug, Default, derive_new::new)]
struct ExchangeLocalKnowledge {
	pub key: Uuid,
	pub target_orders: Vec<Order<PositionOrderId>>,
}

#[instrument(skip(orders_tx, positions_local_knowledge), fields(position_local_knowledge = Empty))]
fn handle_update_from_position(
	hub_rx: PositionToHub,
	positions_local_knowledge: &mut AHashMap<Uuid, PositionLocalKnowledge>,
	orders_tx: &tokio::sync::watch::Sender<HubToExchange>,
	exchanges_local_knowledge: &mut AHashMap<Market, ExchangeLocalKnowledge>,
) -> Result<()> {
	let position_id = hub_rx.position_callback.position_id;
	let position_local_knowledge = positions_local_knowledge
		.entry(position_id)
		.or_insert(PositionLocalKnowledge::new(Uuid::default(), hub_rx.position_callback.sender, Vec::default()));
	Span::current().record("position_local_knowledge", format!("{position_local_knowledge:?}"));

	if position_local_knowledge.key != hub_rx.key {
		// by internal convention, on init the key is Uuid::default()
		debug!("Key mismatch, ignoring the request.");
		return Ok(());
	}
	position_local_knowledge.requested_orders = hub_rx.orders;

	let mut requested_orders_all_positions: Vec<ConceptualOrder<PositionOrderId>> = Vec::default();
	for (position_id, plk) in positions_local_knowledge.iter() {
		let remap_to_position_id = plk.requested_orders.iter().map(|o| {
			let new_id = PositionOrderId::new_from_protocol_id(*position_id, o.id.clone());
			ConceptualOrder { id: new_id, ..o.clone() }
		});
		requested_orders_all_positions.extend(remap_to_position_id);
	}
	let target_orders = hub_process_orders(requested_orders_all_positions);

	debug!(?target_orders);

	// // Binance Futures
	let binance_futures_orders = target_orders
		.iter()
		.filter(|o| o.symbol.market == Market::BinanceFutures)
		.cloned()
		.collect::<Vec<Order<PositionOrderId>>>();

	let exchange_local_knowledge = exchanges_local_knowledge.entry(Market::BinanceFutures).or_default();
	let passforward = HubToExchange::new(exchange_local_knowledge.key, binance_futures_orders);
	orders_tx.send(passforward)?;
	//
	Ok(())
}

#[instrument]
async fn handle_fill(fill: ExchangeToHub, position_local_knowledge: &mut PositionLocalKnowledge) -> Result<()> {
	position_local_knowledge.key = fill.key;
	let vec_fill = vec![ProtocolFill::new(fill.order.id.into(), fill.fill_qty)];
	position_local_knowledge.callback.send(ProtocolFills::new(position_local_knowledge.key, vec_fill)).await?;
	debug!("Sent fills to position");
	Ok(())
}

// HACK
/// Thing that applies all the logic for deciding on how to best express ensemble of requested orders.
#[instrument]
fn hub_process_orders(conceptual_orders: Vec<ConceptualOrder<PositionOrderId>>) -> Vec<Order<PositionOrderId>> {
	let mut orders: Vec<Order<PositionOrderId>> = Vec::default();
	for o in conceptual_orders {
		match &o.order_type {
			ConceptualOrderType::Market(_) => {
				let order = Order::new(o.id, order_types::OrderType::Market, o.symbol.clone(), o.side, o.qty_notional);
				orders.push(order);
			}
			ConceptualOrderType::StopMarket(stop_market) => {
				let order = Order::new(
					o.id,
					order_types::OrderType::StopMarket(order_types::StopMarketOrder::new(stop_market.price)),
					o.symbol.clone(),
					o.side,
					o.qty_notional,
				);
				orders.push(order);
			}
			_ => panic!("Unsupported order type"),
		}
	}
	orders
}

mod tests {
	#![allow(unused_imports)] // RA being dumb
	use order_types::{ConceptualMarket, ConceptualStopMarket};
	use trading_data_core::Side;

	use super::*;
	use crate::exchange_apis::Symbol;

	#[test]
	fn test_hub_process() {
		let from_orders = vec![
			ConceptualOrder {
				id: PositionOrderId::new(Uuid::parse_str("058a3b5d-7ce0-465c-9339-b43261e99b19").unwrap(), "ts:p0.02".to_string(), 0),
				order_type: ConceptualOrderType::Market(ConceptualMarket::default()),
				symbol: Symbol::new("BTC".to_string(), "USDT".to_string(), Market::BinanceFutures),
				side: Side::Buy,
				qty_notional: 100.0,
			},
			ConceptualOrder {
				id: PositionOrderId::new(Uuid::parse_str("86acfda1-ef53-4bae-9f20-bbad6cbc8504").unwrap(), "ts:p0.02".to_string(), 1),
				order_type: ConceptualOrderType::StopMarket(ConceptualStopMarket::default()),
				symbol: Symbol::new("BTC".to_string(), "USDT".to_string(), Market::BinanceFutures),
				side: Side::Buy,
				qty_notional: 100.0,
			},
		];

		let converted = hub_process_orders(from_orders);
		insta::assert_json_snapshot!(converted, @r###"
  [
    {
      "id": {
        "position_id": "058a3b5d-7ce0-465c-9339-b43261e99b19",
        "protocol_id": "ts:p0.02",
        "ordinal": 0
      },
      "order_type": "Market",
      "symbol": {
        "base": "BTC",
        "quote": "USDT",
        "market": "BinanceFutures"
      },
      "side": "Buy",
      "qty_notional": 100.0
    },
    {
      "id": {
        "position_id": "86acfda1-ef53-4bae-9f20-bbad6cbc8504",
        "protocol_id": "ts:p0.02",
        "ordinal": 1
      },
      "order_type": {
        "StopMarket": {
          "price": 0.0
        }
      },
      "symbol": {
        "base": "BTC",
        "quote": "USDT",
        "market": "BinanceFutures"
      },
      "side": "Buy",
      "qty_notional": 100.0
    }
  ]
  "###);
	}
}
