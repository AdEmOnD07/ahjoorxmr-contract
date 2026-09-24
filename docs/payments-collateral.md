# Merchant Collateral in ahjoor-payments

## Overview

ahjoor-payments can require merchants to post collateral before they are allowed to accept open-mode payments. Collateral is denominated in the configured USDC token, held on contract, drawn down to cover customer refunds when a dispute is resolved against a merchant, and never drops below an admin-defined minimum.

The feature is implemented in `contracts/ahjoor-payments/src/lib.rs` and covered by `contracts/ahjoor-payments/src/test_collateral.rs`.

## How the minimum collateral is enforced

The admin sets a global minimum collateral threshold via `set_min_collateral()`. It defaults to `DEFAULT_MIN_COLLATERAL` (1,000,000 — 1 USDC at 7 decimals) and can never be negative.

Minimum collateral is enforced at **merchant approval time** in `approve_merchant()`:

1. The admin calls `approve_merchant(merchant)`.
2. The contract reads the merchant's current collateral balance via `DataKey::MerchantCollateral(merchant)` (defaulting to `0` for merchants that never deposited).
3. If the balance is below the minimum, approval panics with `"Merchant collateral below minimum required"`.
4. Otherwise the merchant is marked approved and can accept payments.

A merchant must therefore deposit at least the minimum (via `deposit_collateral()`) before approval will succeed. Raising the minimum affects future approvals: a merchant who re-applies after the threshold is raised must top up their collateral to the new level.

## Public API

### Admin functions

- `set_min_collateral(min_collateral: i128)` — sets the minimum collateral every merchant must maintain. Panics on negative values.
- `get_min_collateral() -> i128` — returns the current minimum (falls back to the default if unset).

### Merchant functions

- `deposit_collateral(merchant: Address, amount: i128)` — transfers `amount` of the configured USDC token from the merchant into the contract and credits their collateral balance. Amount must be positive.
- `withdraw_collateral(merchant: Address, amount: i128)` — transfers USDC back to the merchant and debits the balance. Blocked if the requested amount exceeds the balance, and blocked if the remaining balance would drop below the minimum.
- `get_collateral_balance(merchant: Address) -> i128` — returns the merchant's current collateral balance (0 for unknown merchants).

## How deposits and withdrawals affect a merchant's standing

- **Deposits** always increase the balance and never require the merchant to be approved yet. Deposits accumulate (`1,000,000` then `500,000` → `1,500,000`) and can be topped up at any time, including after collateral has been slashed.
- **Approval** is only granted when the balance is at least the configured minimum. Setting the minimum to `0` allows approval without any deposit.
- **Withdrawals** are blocked whenever they would take the balance below the minimum, so an approved merchant's standing cannot be eroded below the floor. Withdrawing exactly down to the minimum is allowed.
- **Dispute slashing** reduces the balance when a dispute is resolved in the customer's favour: when the payment token is the collateral token (USDC), collateral is slashed by the owed amount (up to, and capped by, the available balance). Merchants can then top back up via `deposit_collateral()`.

## Storage keys

- `MerchantCollateral(Address)` — persistent per-merchant USDC collateral balance.
- `MinCollateral` — instance-level minimum collateral required for merchant approval.

## Events

- `CollateralDeposited { merchant, amount }` — emitted after a successful deposit.
- `CollateralWithdrawn { merchant, amount }` — emitted after a successful withdrawal.
- `CollateralSlashed { merchant, amount, payment_id }` — emitted when collateral is deducted after a dispute is resolved against the merchant.

## Test coverage

Tests live in `contracts/ahjoor-payments/src/test_collateral.rs`:

- Minimum collateral default, admin configuration, and rejection of negative values.
- Deposit success, accumulation, and rejection of zero/negative amounts.
- Approval gated on collateral: success at/above the minimum, panic with no or insufficient collateral.
- Withdrawal success (leaving at least the minimum), rejection when dropping below the minimum or exceeding the balance.
- Dispute slashing: slash on customer-favoured resolution, no slash on merchant-favoured resolution, slashing capped by available collateral, and no slash for non-USDC payments.
- Per-merchant independence, boundary withdrawals, raising the minimum affecting new approvals, top-up after slashing, and zero minimum allowing approval without a deposit.

## Notes

- Collateral is only accepted in the configured collateral/settlement token (USDC), configured via `set_oracle`.
- Open mode (`set_merchant_open_mode(true)`) bypasses the merchant allowlist entirely; collateral gating applies to merchant approval when the allowlist is enforced.