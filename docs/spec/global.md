# Global (Derived) Invariants
notice that these follow from [fundamental invariants](../ARCHITECTURE.md#invariants)

## Exchange Configuration

r[global.exchanges.static]

Configured exchanges are static for the lifetime of the process. They are initialized once at startup from the config and never change. To update exchange configuration, restart the process. This follows from [crash-only design](../ARCHITECTURE.md#crash-only).
