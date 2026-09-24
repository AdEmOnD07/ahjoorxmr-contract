# Escrow Auto-Renewal

> **Status:** Implemented in `contracts/ahjoor-escrow/src/lib.rs`
> (`create_escrow_with_auto_renew`, `set_renewal_allowance`, `cancel_auto_renew`,
> `cancel_auto_renewal`, `get_renewal_history`, `get_auto_renewal_cancelled`, …)
> and tested in `contracts/ahjoor-escrow/src/test_auto_renewal.rs`.

---

## Overview

**Auto-renewal** enables subscription-style escrows and recurring service agreements in `ahjoor-escrow`. When an escrow configured with auto-renewal is released to the seller, the contract can automatically spin up a new successor escrow with identical commercial terms (buyer, seller, arbiter, token, amount) and a newly calculated deadline without requiring manual contract recreation.

Auto-renewal is designed for:
- Recurring retainer and service agreements
- Subscription-style billing escrows
- Milestone or period extensions where manual renewal is inconvenient

| Fact | Value |
| --- | --- |
| Primary creation entry point | `create_escrow_with_auto_renew` |
| Pre-authorization / allowance | `set_renewal_allowance` / standard token `approve` |
| Cancellation entry points | `cancel_auto_renewal` (recommended / `AutoRenewConfig`), `cancel_auto_renew` (legacy) |
| Query functions | `get_renewal_history`, `get_auto_renewal_cancelled` |
| Storage keys | `DataKey::Escrow(escrow_id)`, `DataKey::RenewalAllowance(escrow_id)`, `DataKey2::AutoRenewalCancelled(escrow_id)`, `DataKey2::RenewalHistory(original_escrow_id)`, `DataKey2::RenewalOriginalId(escrow_id)` |

---

## Escrow Creation with Auto-Renewal

Auto-renewal can be configured during creation using `create_escrow_with_auto_renew`:

```rust
pub fn create_escrow_with_auto_renew(
    env: Env,
    buyer: Address,
    seller: Address,
    arbiter: Address,
    amount: i128,
    token: Address,
    deadline: u64,
    metadata_hash: Option<BytesN<32>>,
    auto_renew_config: AutoRenewConfig,
) -> u32
```

The `AutoRenewConfig` struct specifies:
- `max_renewals: u32`: The maximum number of renewal cycles permitted (e.g. `3` allows up to 3 automatic renewals after the initial escrow). If set to `0`, auto-renewal will not trigger.
- `renewal_interval_ledgers: u32`: Duration for each renewal period measured in ledger sequence steps (approximately 5 seconds per ledger).

When created:
1. `buyer.require_auth()` is verified.
2. Initial escrow funds (`amount`) are immediately transferred from the buyer to the contract.
3. The escrow status is set to `EscrowStatus::Active` and `renewals_completed` is initialized to `0`.

---

## Auto-Renewal Lifecycle

```text
               +--------------------------------------------------+
               | buyer calls create_escrow_with_auto_renew(...)   |
               | - Transfers initial amount to contract           |
               | - Stores AutoRenewConfig (max_renewals, interval)|
               +--------------------------------------------------+
                                        |
                                        v
               +--------------------------------------------------+
               | buyer pre-authorizes funds:                      |
               | - set_renewal_allowance(...) or token approve    |
               +--------------------------------------------------+
                                        |
                 +----------------------+----------------------+
                 |                                              |
                 v (cancel_auto_renewal called)                 v (no cancellation)
      +-----------------------------+               +-----------------------------+
      | AutoRenewalCancelled = true |               | Buyer or Arbiter releases   |
      +-----------------------------+               | escrow (release_escrow)     |
                 |                                  +-----------------------------+
                 v                                              |
      +-----------------------------+                           v
      | release_escrow pays seller  |               +-----------------------------+
      | Auto-renewal is SKIPPED     |               | Funds paid to seller        |
      +-----------------------------+               | Contract checks auto-renew  |
                                                    +-----------------------------+
                                                                |
                                    +---------------------------+---------------------------+
                                    |                                                       |
                                    v (allowance available &                                v (insufficient allowance)
                                       renewals_completed < max)                            |
                         +-----------------------------------+             +-----------------------------------+
                         | 1. Pulls amount via transfer_from |             | 1. Emits RenewalFailed event      |
                         | 2. Creates new successor escrow   |             | 2. Skips renewal gracefully       |
                         | 3. Increments renewals_completed  |             +-----------------------------------+
                         | 4. Appends to RenewalHistory      |
                         | 5. Emits EscrowAutoRenewedV2      |
                         +-----------------------------------+
```

### Triggering Renewals on Release

Auto-renewals are evaluated inside `release_escrow` after the current escrow has been marked `EscrowStatus::Released` and existing funds have been disbursed to the seller:

1. **Cancellation Check:** If `DataKey2::AutoRenewalCancelled(escrow_id)` is `true`, renewal is skipped.
2. **Cap Check:** If `renewals_completed >= max_renewals`, renewal is skipped.
3. **Funding Transfer:** The contract invokes `token_client.try_transfer_from(...)` to pull `amount` from the buyer.
   - If the transfer fails (e.g. insufficient allowance or balance), the contract emits a `RenewalFailed` event with reason `"InsufficientAllowance"` and completes release without reverting.
4. **Successor Creation:** A new escrow ID is allocated with:
   - Identical `buyer`, `seller`, `arbiter`, `token`, `amount`, and `metadata_hash`.
   - New deadline calculated as: `current_timestamp + (renewal_interval_ledgers * 5)`.
   - `renewals_completed` incremented by 1 (`renewal_index = renewals_completed + 1`).
   - `AutoRenewConfig` preserved for further cycles.
5. **History Tracking:** The new escrow ID is appended to `DataKey2::RenewalHistory(original_id)` and mapped via `DataKey2::RenewalOriginalId(new_escrow_id) = original_id`.
6. **Event Emission:** `EscrowAutoRenewedV2` is emitted.

> [!NOTE]
> Escrows resolved via dispute refund or dispute resolution do **not** trigger auto-renewal. Renewal only executes upon a normal successful `release_escrow`.

---

## Capping Renewals & Renewal Allowance

To ensure buyers retain complete financial control and prevent unbounded fund transfers, auto-renewal incorporates multi-layered authorization caps:

### 1. `max_renewals` Configuration Cap
Configured at creation in `AutoRenewConfig`. The contract guarantees that no more than `max_renewals` successor escrows will be created across the entire renewal chain.

### 2. `set_renewal_allowance` Entry Point
The buyer can explicitly set the number of renewals pre-authorized and grant the required token approval to the contract:

```rust
pub fn set_renewal_allowance(env: Env, buyer: Address, escrow_id: u32, total_renewals: u32)
```

**Authentication:** `buyer.require_auth()`

**Preconditions & Rules:**
- Caller must be the escrow buyer (`EscrowError::OnlyBuyerCanSetRenewalAllowance`).
- Escrow must have auto-renewal enabled (`EscrowError::AutoRenewIsNotEnabledEscrow`).
- Calculates `total_amount = escrow.amount * total_renewals`.
- Approves the contract address to spend `total_amount` from the buyer with an expiration ledger.
- Stores `RenewalAllowance(escrow_id) = total_renewals`.

---

## Stopping Future Renewals (`cancel_auto_renew` & `cancel_auto_renewal`)

A buyer can stop future renewals at any point before release.

### Recommended: `cancel_auto_renewal`

```rust
pub fn cancel_auto_renewal(env: Env, buyer: Address, escrow_id: u32)
```

**Authentication:** `buyer.require_auth()`

**Behavior:**
1. Verifies caller is the buyer (`EscrowError::OnlyBuyerCanCancelAutoRenewal`).
2. Validates that the escrow was configured with `AutoRenewConfig` (`EscrowError::NoAutoRenewConfigSetEscrow`).
3. Sets `DataKey2::AutoRenewalCancelled(escrow_id) = true`.
4. Emits `AutoRenewalCancelled` event.
5. Can be called on either the initial escrow or any successor escrow in the chain. When the current escrow is released, no successor escrow will be spawned.
6. The cancellation call is idempotent and safe to call multiple times.

### Legacy: `cancel_auto_renew`

```rust
pub fn cancel_auto_renew(env: Env, buyer: Address, escrow_id: u32)
```

**Authentication:** `buyer.require_auth()`

**Behavior:**
- Sets `escrow.extensions.auto_renew = false` and `escrow.extensions.renewals_remaining = 0`.
- Removes `DataKey::RenewalAllowance(escrow_id)`.

---

## Querying Renewal State & History

### Query Successor Chain

```rust
pub fn get_renewal_history(env: Env, escrow_id: u32) -> Vec<u32>
```
Returns an ordered `Vec<u32>` of successor escrow IDs generated from the original escrow. Returns an empty vector if no renewals occurred.

### Query Cancellation Status

```rust
pub fn get_auto_renewal_cancelled(env: Env, escrow_id: u32) -> bool
```
Returns `true` if `cancel_auto_renewal` was invoked for the given `escrow_id`, and `false` otherwise.

---

## Events

| Event | Topic / Emitted By | Payload / Fields | Description |
| --- | --- | --- | --- |
| `EscrowAutoRenewedV2` | `try_auto_renew` | `(old_escrow_id, new_escrow_id, renewal_index)` | Emitted when a renewal cycle successfully creates a new escrow. |
| `RenewalFailed` | `try_auto_renew` | `(escrow_id, renewal_index, reason)` | Emitted when renewal fails (e.g. `"InsufficientAllowance"`). |
| `AutoRenewalCancelled` | `cancel_auto_renewal` | `(escrow_id)` | Emitted when buyer cancels future auto-renewals. |
| `EscrowAutoRenewed` | Legacy renewal | `(old_escrow_id, new_escrow_id, renewals_remaining)` | Legacy renewal emission. |

---

## Error Codes

| Code | Name | Description |
| --- | --- | --- |
| 45 | `OnlyBuyerCanSetRenewalAllowance` | Non-buyer tried to call `set_renewal_allowance`. |
| 46 | `AutoRenewIsNotEnabledEscrow` | Escrow does not have auto-renewal enabled. |
| 47 | `OnlyBuyerCanCancelAutoRenew` | Non-buyer tried to call legacy `cancel_auto_renew`. |
| 48 | `OnlyBuyerCanCancelAutoRenewal` | Non-buyer tried to call `cancel_auto_renewal`. |
| 49 | `NoAutoRenewConfigSetEscrow` | `cancel_auto_renewal` called on an escrow without `AutoRenewConfig`. |
| Ext4(14) | `InsufficientRenewalAllowance` | Legacy renewal allowance is zero. |
| Ext4(15) | `EscrowRenewalDurationMustBePositive` | Renewal duration calculation resulted in 0. |

---

## Function Reference

| Function | Caller | Purpose |
| --- | --- | --- |
| `create_escrow_with_auto_renew(...)` | Buyer | Creates a new escrow configured with `AutoRenewConfig`. |
| `set_renewal_allowance(buyer, escrow_id, total_renewals)` | Buyer | Pre-authorizes allowance and sets token spending approval for renewals. |
| `cancel_auto_renewal(buyer, escrow_id)` | Buyer | Cancels future renewals for escrows configured with `AutoRenewConfig`. |
| `cancel_auto_renew(buyer, escrow_id)` | Buyer | Disables legacy auto-renewal on an escrow. |
| `get_renewal_history(escrow_id)` | Anyone | Retrieves the list of all successor escrow IDs spawned in the chain. |
| `get_auto_renewal_cancelled(escrow_id)` | Anyone | Checks if future auto-renewals have been cancelled for the escrow. |

---

## Storage Reference

| Key | Storage Type | Data Stored |
| --- | --- | --- |
| `DataKey::Escrow(escrow_id)` | Persistent | Escrow record containing status, parties, amount, and `EscrowExtensions` (e.g. `auto_renew_max_renewals`, `auto_renew_interval_ledgers`, `renewals_completed`). |
| `DataKey::RenewalAllowance(escrow_id)` | Persistent | `u32` representing remaining pre-authorized renewals. |
| `DataKey2::AutoRenewalCancelled(escrow_id)` | Persistent | `bool` flag marking whether buyer cancelled auto-renewal. |
| `DataKey2::RenewalHistory(original_escrow_id)` | Persistent | `Vec<u32>` of successor escrow IDs in chronological order. |
| `DataKey2::RenewalOriginalId(escrow_id)` | Persistent | `u32` mapping a renewed successor escrow ID back to its original root escrow ID. |
