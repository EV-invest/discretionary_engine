//TODO: update
# Architecture
```mermaid
flowchart TD
  Hub["Hub"]

  subgraph positions ["POSITIONS"]
    subgraph cluster_position_1 ["Position I"]
      Protocol1_Params1 --> S
      Protocol3_Params1 --> S

      F --> |"apply fill mask on ProtocolOrders
      objects protocols are sending,
      and refresh current suggested
      orders on Position"| S

      S["All suggested orders for this Position"]

      F["Fill port of the Position"]
    end
    PositionII["Position II"]
    PositionIII["Position III"]
  end
  PositionII --> Hub
  PositionIII --> Hub
  S --> |"Knowing how much
  each protocol manages,
  convert suggested orders,
  (size as % of total under
  protocol's management),
  into notional sizes.
  After choose up to
  target position size
  from them, so as to not
  risk having additional
  stale exposure"| Hub
  Hub -->|"fill"| F

  subgraph cluster_exchanges ["Exchange API modules"]
    direction TB
    BinanceFutures
    BinanceSpot
    BybitFutures
    Coinbase
  end

  Hub --> BinanceFutures
  Hub --> BinanceSpot
  Hub --> BybitFutures
  Hub --> Coinbase

  subgraph cluster_position_1_fills
    direction TB
    BinanceFutures -.->|"fill"| cluster_position_1
    BinanceSpot -.->|"fill"| cluster_position_1
    BybitFutures -.->|"fill"| cluster_position_1
    Coinbase -.->|"fill"| cluster_position_1
  end
  
  subgraph cluster_position_2_fills
    direction TB
    BinanceFutures -.->|"fill"| PositionII
    BinanceSpot -.->|"fill"| PositionII
    BybitFutures -.->|"fill"| PositionII
    Coinbase -.->|"fill"| PositionII
  end
  
  subgraph cluster_position_3_fills
    direction TB
    BinanceFutures -.->|"fill"| PositionIII
    BinanceSpot -.->|"fill"| PositionIII
    BybitFutures -.->|"fill"| PositionIII
    Coinbase -.->|"fill"| PositionIII
  end

  style BinanceFutures fill:none,stroke:#000,stroke-width:1,stroke-dasharray: 5
  style BinanceSpot fill:none,stroke:#000,stroke-width:1,stroke-dasharray: 5
  style BybitFutures fill:none,stroke:#000,stroke-width:1,stroke-dasharray: 5
  style Coinbase fill:none,stroke:#000,stroke-width:1,stroke-dasharray: 5
```

### `main.rs`
Cli entry

### `positions.rs`
Main acting element. Every exposed api function of the crate creates new position or edits an old one.

Position takes in _what it should be_, then figures out execution strategy for getting there. By default positions open the position being mindful of the market impact and best exchange from standpoint of latency and commissions, and that's it. For more control of execution details we expose [protocols](#protocols)) and split Position into `Acquisition` and `Followup` parts.

All Positions post updates of their desired outstanding orders to the [hub](#exchange-apis), when they decide on altering their executing strategy based on some new market data or information of fills on orders owned by them.

### `protocols/`
Way to control specifics of execution of the Position they are assigned to. Think of them as additional configuration settings for the Position. For more fine-grained control, protocols are split to `Acquisition` and `Followup`, specified for according stage of their Position's lifetime.

Protocols analyze any kind of market information relevant to the position they are attached to, and output their suggestion of their Position's behavior as they think is appropriate for the situation. Position then gives a predefined weight to the suggestions of each protocol, and joins them with those of others before deciding on which of the suggested orders it will be passing to Hub for execution.

All available protocols are predefined, and an api for manual on-demand creation of specific protocols from common market data is not currently planned.

### `exchange_apis/`
Interface through Hub, meant to synchronize information of all outstanding to allow for more efficient rerouting (considering e.g., current balances on exchanges). Hub keeps track of the _last_ sent Orders update from each Position, dynamically deciding how exactly and on which exchanges they should be executed, having ability to move them around.

Exchange interfaces themselves keep track only of the orders given to them by the Hub, and mostly deal with specifics of api of their respective exchange. No order changes transpire at this level, they are placed exactly as directed by the Hub.

### Communication
when sending or receiving orders every actor attaches a `last_fill_key`. It must match the last key attached to the latest report to this actor by however handles execution of its requests. It's used to ensure that all client's requests are based on the up-to-date knowledge of the relevant state. By internal convention, if the client is yet to receive any reports, it sends `Uuid::default()`.

# Design Principles
- [Disruptors infrastructure](https://martinfowler.com/articles/lmax.html), with clear separation of _pure_ business logic (sequential in nature), from networking-bound communications for both {receiving data, communicating order execution} (asynchronous in nature)
