use derive_more::Deref;
use strum::{AsRefStr, Display, EnumIter, EnumString, FromRepr};
use v_exchanges::ExchangeOrder;

/// Lifecycle state of an order on the venue.
///
/// An order is considered _open_ for:
///  - `Accepted`, `PendingUpdate`, `PendingCancel`, `PartiallyFilled`
///    // we're omitting `Triggered`, as we purposefuly operate exclusively with normal limits
///
/// An order is considered _in-flight_ for:
///  - `Submitted`, `PendingUpdate`, `PendingCancel`
///
/// An order is considered _closed_ for:
///  - `Denied`, `Rejected`, `Canceled`, `Expired`, `Filled`
#[repr(C)]
#[derive(AsRefStr, Clone, Copy, Debug, Default, Display, EnumIter, EnumString, Eq, FromRepr, Hash, Ord, PartialEq, PartialOrd)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderState {
	/// The order is initialized within the system but not yet submitted.
	#[default]
	Initialized,
	/// The order was denied before reaching the venue.
	// eg we're in system-wide `reduce-only` mode, and it would've violated it
	Denied,
	/// The order was submitted to the venue (awaiting acknowledgement).
	Submitted,
	/// The order was acknowledged by the venue as valid.
	Accepted,
	/// The order was rejected by the venue.
	// eg when we're outside of the %price bounds
	Rejected,
	/// The order was canceled.
	Canceled,
	/// The order reached GTD expiration.
	Expired,
	/// The order is pending a modify request on the venue.
	PendingUpdate,
	/// The order is pending a cancel request on the venue.
	PendingCancel,
	/// The order has been partially filled.
	PartiallyFilled,
	/// The order has been completely filled.
	Filled,
}

impl OrderState {
	pub fn is_open(self) -> bool {
		matches!(self, Self::Accepted | Self::PendingUpdate | Self::PendingCancel | Self::PartiallyFilled)
	}

	pub fn is_in_flight(self) -> bool {
		matches!(self, Self::Submitted | Self::PendingUpdate | Self::PendingCancel)
	}

	pub fn is_closed(self) -> bool {
		matches!(self, Self::Denied | Self::Rejected | Self::Canceled | Self::Expired | Self::Filled)
	}
}

/// An [`ExchangeOrder`] extended with execution tracking state.
///
/// `PartialEq` covers only the pre-compiled (inner) order definition — `__filled` and `state` are excluded.
#[derive(Clone, Debug, Deref, derive_new::new)]
pub struct ExecOrder<O> {
	#[deref]
	pub inner: ExchangeOrder<O>,
	#[new(default)]
	__filled: f64,
	#[new(default)]
	pub state: OrderState,
}

impl<O: PartialEq> PartialEq for ExecOrder<O> {
	fn eq(&self, other: &Self) -> bool {
		self.inner == other.inner
	}
}
