# Multi-Party Approval in `ahjoor-escrow`

This document details the N-of-M multi-party approval feature in `ahjoor-escrow`, which enables decentralized sign-off before escrow funds can be released to the seller.

---

## 1. Overview

In high-value or governance-governed escrow arrangements, relying on a single buyer or arbiter to release funds may introduce counterparty risk or operational bottlenecks. The multi-party approval feature introduces an on-chain N-of-M threshold authorization scheme for release.

Under this model:
- The **buyer** configures a designated set of trusted approvers ($M$) and a required approval threshold ($N$).
- Each authorized approver independently calls `approve_release` to register their sign-off.
- The contract tallies approvals on-chain. As soon as the number of unique approvals reaches the configured threshold ($N$), the contract **automatically releases the escrowed funds to the seller**, returns any seller collateral, burns any associated receipt NFT, and marks the escrow as `Released`.

---

## 2. Configuring Multi-Party Approval (`set_multiparty_approval`)

Multi-party approval is configured per escrow by the buyer after an escrow has been created and funded.

### 2.1 Function Signature

```rust
pub fn set_multiparty_approval(
    env: Env,
    buyer: Address,
    escrow_id: u32,
    approvers: Vec<Address>,
    threshold: u32,
)
```

### 2.2 Parameters

| Parameter | Type | Description |
|---|---|---|
| `buyer` | `Address` | Address of the escrow buyer (must authenticate the transaction). |
| `escrow_id` | `u32` | Unique identifier of the target escrow. |
| `approvers` | `Vec<Address>` | List of distinct addresses authorized to vote for fund release (between 2 and 10 addresses). |
| `threshold` | `u32` | Minimum number of approvals required to execute fund release ($1 \le \text{threshold} \le \text{approvers.len()}$). |

### 2.3 Rules and Validation

When `set_multiparty_approval` is invoked, the contract strictly enforces the following requirements:

1. **Pause Check**: The contract must not be paused (`require_not_paused`).
2. **Buyer Authentication**: The `buyer` address must authorize the invocation via `buyer.require_auth()`.
3. **Escrow Existence**: An escrow record must exist under `DataKey::Escrow(escrow_id)`.
4. **Buyer Authorization**: Only the recorded buyer of the escrow may configure approvals. Any other caller is rejected with `EscrowErrorExt3::OnlyBuyerCanConfigureMultiPartyApproval` (error code `43`).
5. **Active Status**: The escrow must be in an open state (`Active`, `PartiallyReleased`, or `InspectionPassed`). Invocations on completed, refunded, disputed, or otherwise non-open escrows fail with `EscrowError::EscrowIsNotActive` (error code `3`).
6. **Approver Count Constraints**: The approver set must contain between 2 and 10 addresses inclusive:
   $$2 \le \text{approvers.len()} \le 10$$
   Violating this boundary fails with `EscrowErrorExt3::ApproversCountMustBeBetween2And10` (error code `44`).
7. **Threshold Constraints**: The threshold must be strictly greater than zero and cannot exceed the total number of approvers:
   $$1 \le \text{threshold} \le \text{approvers.len()}$$
   Invalid thresholds fail with `EscrowErrorExt3::ThresholdMustBeBetween1AndApproversCount` (error code `45`).
8. **Immutability Once Approvals Begin**: If any approver has already submitted an approval (`ReleaseApprovals(escrow_id)` is non-empty), re-configuration is blocked with `EscrowErrorExt3::CannotReconfigureApprovalsAlreadyProgress` (error code `46`). This prevents the buyer from altering the approver list or threshold mid-vote.

### 2.4 State Storage

When configuration succeeds, the contract persists:
- `DataKey2::MultiPartyApprovers(escrow_id)`: The vector of approved addresses.
- `DataKey2::MultiPartyThreshold(escrow_id)`: The `u32` threshold value.
- `DataKey2::ReleaseApprovals(escrow_id)`: Initialized as an empty `Vec<Address>`.

All corresponding persistent storage keys and instance storage TTLs are extended via `extend_ttl`.

---

## 3. Submitting Approvals (`approve_release`)

Designated approvers submit their sign-off using `approve_release`.

### 3.1 Function Signature

```rust
pub fn approve_release(
    env: Env,
    approver: Address,
    escrow_id: u32,
)
```

### 3.2 Caller Requirements and Execution Flow

1. **Authentication**: The caller must authenticate as `approver` via `approver.require_auth()`.
2. **Contract State**: The contract must not be paused.
3. **Open Escrow**: The escrow must be in an open state (`is_open_escrow_status`). If the escrow is disputed, released, refunded, or abandoned, the call fails with `EscrowIsNotActive` (error code `3`).
4. **Authorized Approver**: The caller must exist in `DataKey2::MultiPartyApprovers(escrow_id)`. Non-approvers fail with `EscrowErrorExt3::CallerIsNotAuthorizedApproverEscrow` (error code `47`).
5. **No Duplicate Votes**: Each approver can only vote once. If the address is already recorded in `DataKey2::ReleaseApprovals(escrow_id)`, the transaction reverts with `EscrowErrorExt3::ApproverHasAlreadyApprovedEscrow` (error code `48`).
6. **Approval Recorded**: The approver's address is appended to `DataKey2::ReleaseApprovals(escrow_id)`.
7. **Event Emitted**: A `MultiPartyApproval` event is published containing `(escrow_id, approver, approvals_count, threshold)`.

### 3.3 Automatic Threshold Execution

If the updated `approvals_count` meets or exceeds `threshold`:

1. **Payment Transfer**: The contract calls `Self::transfer_to_sellers(&env, &escrow, total, escrow_id)`, transferring the full escrow amount to the seller (or distributing across multiple sellers if multi-seller).
2. **Seller Collateral Return**: If the seller deposited collateral (`DataKey::SellerCollateral(escrow_id)` > 0), the collateral is transferred back to the seller, an `EscrowCollateralReturned` event is emitted, and the collateral storage record is cleared.
3. **Status Update**: Escrow status transitions to `EscrowStatus::Released`. The status change is recorded in history via `record_status_history`.
4. **Receipt NFT**: If a receipt NFT was minted for this escrow, it is burned via `burn_receipt_if_exists`.
5. **Persistence**: The updated `Escrow` struct is saved back to storage and TTLs are bumped.

---

## 4. Query Functions

The contract exposes dedicated read-only functions to inspect multi-party approval configuration and voting status.

### 4.1 `get_multiparty_config`

Retrieves the approvers list and required threshold for an escrow.

```rust
pub fn get_multiparty_config(env: Env, escrow_id: u32) -> Option<(Vec<Address>, u32)>
```

- **Returns**:
  - `Some((approvers, threshold))` if multi-party approval has been configured for the specified `escrow_id`.
  - `None` if `set_multiparty_approval` has never been called on the escrow.

### 4.2 `get_release_approvals`

Retrieves the list of addresses that have approved release so far.

```rust
pub fn get_release_approvals(env: Env, escrow_id: u32) -> Vec<Address>
```

- **Returns**: A vector of `Address` values representing all approvers who have successfully called `approve_release`.
- Returns an empty vector `[]` if no approvals have been registered.

---

## 5. Interactions with Normal Release and Dispute Flows

Multi-party approval works in harmony with existing escrow flows while maintaining strict safety guarantees:

### 5.1 Direct Release (`release_escrow`)

- `release_escrow` is the standard direct release endpoint callable by the buyer or the assigned arbiter.
- Multi-party approval serves as a threshold-governed alternative to direct release.
- If a buyer or arbiter directly executes `release_escrow`, the escrow transitions to `EscrowStatus::Released`.
- Once in `Released` status, the escrow is in a terminal state. Any subsequent attempt to call `approve_release` or `set_multiparty_approval` fails with `EscrowError::EscrowIsNotActive` because `is_open_escrow_status` evaluates to `false`.

### 5.2 Dispute Flow (`dispute_escrow`)

- While approvers are reviewing and voting, either party (buyer or seller) retains the right to raise a dispute via `dispute_escrow`.
- When a dispute is raised, the escrow status changes to `EscrowStatus::Disputed` (or `EscrowStatus::PartiallyDisputed`).
- **Voting Frozen During Dispute**: While in `Disputed` status, `is_open_escrow_status` returns `false`. Consequently, calls to `approve_release` are blocked and will revert with `EscrowIsNotActive`.
- **Dispute Resolution**: Once the arbiter issues a ruling via `resolve_dispute` (or a dispute timeout is enforced via `enforce_dispute_timeout`), funds are distributed and the escrow enters a terminal status (`EscrowStatus::Resolved`, `EscrowStatus::Released`, or `EscrowStatus::Refunded`). No further multi-party approvals can be submitted once resolved.

### 5.3 Inspection Gate (`seller_mark_complete` / `submit_inspection_result`)

- If an inspector is configured for the escrow:
  - When the seller calls `seller_mark_complete`, the escrow enters `EscrowStatus::AwaitingInspection`.
  - Approvals cannot be submitted while awaiting inspection (`is_open_escrow_status` returns `false`).
  - If the inspector approves the report, the escrow moves to `EscrowStatus::InspectionPassed`.
  - `InspectionPassed` is treated as an open status (`is_open_escrow_status` returns `true`), unblocking approvers to cast their votes via `approve_release`.

### 5.4 Seller Collateral

- If the escrow required seller collateral, it remains locked in the contract throughout the voting process.
- Upon reaching the approval threshold, collateral is automatically returned to the seller alongside the primary fund disbursement.

---

## 6. Events Reference

The following contract events are emitted during the multi-party approval lifecycle:

| Event | Topics / Fields | Description |
|---|---|---|
| `MultiPartyApproval` | `escrow_id: u32`<br>`approver: Address`<br>`approvals_count: u32`<br>`threshold: u32` | Emitted every time an authorized approver signs off on release. |
| `EscrowCollateralReturned` | `escrow_id: u32`<br>`seller: Address`<br>`amount: i128` | Emitted when threshold is reached and seller collateral is refunded. |

---

## 7. Error Reference

Multi-party approval operations surface specific error codes from `EscrowError` and `EscrowErrorExt3`:

| Code | Error Name | Cause |
|---|---|---|
| `3` | `EscrowIsNotActive` | Operation attempted on an escrow that is not in an open status (`Active`, `PartiallyReleased`, or `InspectionPassed`). |
| `43` | `OnlyBuyerCanConfigureMultiPartyApproval` | `set_multiparty_approval` called by an account other than the escrow buyer. |
| `44` | `ApproversCountMustBeBetween2And10` | `set_multiparty_approval` called with fewer than 2 or more than 10 approvers. |
| `45` | `ThresholdMustBeBetween1AndApproversCount` | Threshold specified is `0` or exceeds the size of the approvers vector. |
| `46` | `CannotReconfigureApprovalsAlreadyProgress` | `set_multiparty_approval` called after one or more approvals have already been recorded. |
| `47` | `CallerIsNotAuthorizedApproverEscrow` | `approve_release` called by an address not present in the approver list. |
| `48` | `ApproverHasAlreadyApprovedEscrow` | `approve_release` called more than once by the same approver. |

---

## 8. Example Workflow

### Step 1: Create and Fund Escrow
Buyer creates an escrow locking 1,000 XLM for a seller:
```bash
# Escrow ID 42 created with buyer Alice and seller Bob
```

### Step 2: Configure 2-of-3 Multi-Party Approval
Buyer Alice designates three auditors (`Carol`, `Dave`, `Eve`) with a threshold of 2:
```bash
stellar contract invoke \
  --id <ESCROW_CONTRACT_ID> \
  --source alice \
  --network testnet \
  -- set_multiparty_approval \
  --buyer alice \
  --escrow_id 42 \
  --approvers '["CAROL_ADDRESS", "DAVE_ADDRESS", "EVE_ADDRESS"]' \
  --threshold 2
```

### Step 3: Query Configuration
Inspect on-chain configuration:
```bash
stellar contract invoke \
  --id <ESCROW_CONTRACT_ID> \
  --network testnet \
  -- get_multiparty_config \
  --escrow_id 42
# Returns: (["CAROL_ADDRESS", "DAVE_ADDRESS", "EVE_ADDRESS"], 2)
```

### Step 4: First Approval
Carol signs off on the release:
```bash
stellar contract invoke \
  --id <ESCROW_CONTRACT_ID> \
  --source carol \
  --network testnet \
  -- approve_release \
  --approver carol \
  --escrow_id 42
# Approvals count: 1 / 2. Funds remain in escrow.
```

### Step 5: Second Approval (Threshold Met)
Dave submits the second approval:
```bash
stellar contract invoke \
  --id <ESCROW_CONTRACT_ID> \
  --source dave \
  --network testnet \
  -- approve_release \
  --approver dave \
  --escrow_id 42
# Approvals count: 2 / 2. Threshold met!
# Contract automatically transfers 1,000 XLM to Bob and updates status to Released.
```
