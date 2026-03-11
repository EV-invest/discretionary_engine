## `_routing`
operates over compiled intent.

### Invariants
- input's thin waist is exclusively through `ConceptualOrder`.

- outputs **exact orders**, associated with exact exchange

- generated orders contain both {actual price, expected fee}.
  this allows us to cancel against ourselves. As our architecture allows unlimited number of delta-change processes running on the same asset. And note that most of the time if we have 2 opposite sides `ConceptualOrder`s running, their target limits will likely not touch, so if not for `expected fee`, canceling out would not even be a thing.
  ![here is a visual example](../assets/doodle_internal_order_cancellation.png)

- directly takes care of executing and retrying for orders.
  If an order is not being passed, - it should determine if it's the fault of the exchange, or if something more general, and then handle it.


# Design Principles
- [Disruptors infrastructure](https://martinfowler.com/articles/lmax.html), with clear separation of _pure_ business logic (sequential in nature), from networking-bound communications for both {receiving data, communicating order execution} (asynchronous in nature)
