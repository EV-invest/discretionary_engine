# Protocol System Specification

## Order Identity

r[protocol.orders.id-stability]

A protocol MUST maintain stable order IDs across its lifetime. The ordinal index in `ProtocolOrders.__orders` for a given logical order MUST remain constant across all updates.

r[protocol.orders.id-match]

The `ProtocolOrderId` attached to each order MUST match the protocol's signature and the order's ordinal position in the orders vector.

r[protocol.orders.all-slots-initialized]

A protocol MUST always send a `Vec<Option<ConceptualOrderPercents>>` with slots for ALL possible orders it may ever request. Orders not currently active MUST be represented as `None`, not omitted.

## Order Processing

r[protocol.orders.market-first]

When processing orders from protocols, market-like orders MUST be executed before other order types.
