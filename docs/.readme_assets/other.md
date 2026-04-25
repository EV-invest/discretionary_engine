## Assumptions
- strictly one asset per `Position`. No methods for acquiring several assets at once will ever be introduced.

### Current (may change in the future)
- no two `Position`s are opened on the same symbol

- no new `Position`s on account are opened outside of the engine

- orders are placed immediately (not that far off, as most of the time we will spam the thing until it accepts. And only other action, that will need to be taken, is to prevent any increases in exposure while we have any mismatches).

## Roadmap
- [ ] micro/macro data distinctions
- [ ] scale to multiple positions
    if we're correctly using websockets for trading, actually don't think we need to have a mother program for all to share exchange connections: micro data will be stored on a separate server; macro is generally cheap to pull (or needs to be pre-compiled anyways)
