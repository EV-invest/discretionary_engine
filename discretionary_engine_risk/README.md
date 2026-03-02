# discretionary_engine_risk

Risk management library for `discretionary_engine`.

## Usage

```sh
discretionary_engine risk size binance:btc-usdt -q c --percent-sl 2%
discretionary_engine risk balance
```

Config is in `~/.config/discretionary_engine.nix` under the `risk` section. Example:
```nix
{
  # ... other discretionary_engine config ...

  risk = {
    # other_balances = { coinpoker = 75; polymarket = 100; };  # balances not tracked on exchanges
    size = {
      default_sl = 0.02;
      round_bias = "5%";
      abs_max_risk = "20%";
      risk_layers = {
        stop_loss_proximity = true;
      };
    };
  };
}
```
