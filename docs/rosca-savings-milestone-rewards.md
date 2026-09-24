# ROSCA Savings Goal — Milestone Rewards

## Overview

The savings milestone rewards system lets ROSCA members attach on-chain token rewards to their savings goals. When a member's cumulative contributions cross a defined percentage threshold, the contract automatically transfers a reward from a pre-funded reward pool to the member's wallet. No extra transaction is required from the member.

This feature lives in `contracts/ahjoor-rosca/src/savings_goal_tracking_impl.rs` and is tested in `contracts/ahjoor-rosca/src/test_savings_milestone_rewards.rs`.

---

## How Milestones Are Defined on a Goal

Milestones are added to a goal after creation via `add_savings_goal_milestones`. Each `Milestone` struct carries these fields relevant to rewards:

```rust
pub struct Milestone {
    pub milestone_id: u32,     // Unique ID within the goal
    pub percentage:   u32,     // Progress threshold (1–100) that triggers the reward
    pub amount:       i128,    // Informational target amount (must be > 0)
    pub reward_bps:   u32,     // Basis points of contribution amount paid as reward
                               // 0 = no token reward; 10_000 = 100% of contribution
    // ... name, description, reward_type, reward_value, celebration_event
}
```

Key constraints validated on `add_milestones`:

- `percentage` must be 1–100 (inclusive).
- `amount` must be positive.
- `reward_bps` of `0` means the milestone generates no token transfer (celebration-only).
- Multiple milestones at different percentage thresholds can coexist on the same goal.

Example — add a 25 % milestone that pays 10 % of the triggering contribution:

```rust
client.add_savings_goal_milestones(&goal_id, &vec![
    Milestone {
        milestone_id: 1,
        percentage: 25,
        amount: 250,
        reward_bps: 1_000,   // 10% of contribution amount
        reward_type: RewardType::Bonus,
        // ...
    },
]);
```

---

## When Rewards Are Distributed

### Automatic — on every contribution

`check_and_distribute_milestone_rewards` is called internally at the end of `contribute_to_savings_goal`. You do not need to call it yourself.

The logic for each milestone with `reward_bps > 0`:

1. Compute the progress percentage before and after the contribution.
2. If the threshold was **not** crossed before but **is** crossed now, the milestone fires.
3. Reward amount: `contribution_amount × reward_bps / 10_000`.
4. If the `SavingsRewardPool` holds enough tokens, `token::Client::transfer` moves the reward from the contract to the member.
5. If the pool is depleted (balance < reward), the transfer is **skipped silently** — the transaction does not revert.
6. In both cases the milestone's bit is set in the per-member bitmask (`SavingsMilestonesClaimed`), preventing the same milestone from firing again.

### Pool funding (admin action)

The reward pool must be funded before rewards can be paid. The admin calls:

```rust
client.fund_savings_reward_pool(&admin, &amount);
```

The pool balance is queryable at any time:

```rust
let pool: i128 = client.get_savings_reward_pool();
```

### Bitmask deduplication

Each goal+member pair has a `u64` bitmask stored under `DataKey3::SavingsMilestonesClaimed(goal_id, member)`. Bit position `milestone_id % 64` is set once the milestone fires. This guarantees each milestone reward is paid **at most once**, even if subsequent contributions keep the percentage above the threshold.

```rust
// Check programmatically:
let bitmask: u64 = client.get_savings_milestones_claimed(&goal_id, &member);
let is_claimed = bitmask & (1u64 << (milestone_id % 64)) != 0;
```

---

## Celebrations vs. Reward Issuance

These are two separate concerns with different call paths:

| Concern | Function | When called | What it does |
|---|---|---|---|
| Automatic reward | `check_and_distribute_milestone_rewards` | Inside `contribute_to_savings_goal` | Transfers tokens from pool, emits `milestone_reached` event |
| Celebration record | `check_and_celebrate_milestones` | Must be called explicitly | Creates a `MilestoneCelebration` record, updates `completed_milestones` list on goal |
| Manual celebration | `celebrate_milestone` | Called by member | Creates a `MilestoneCelebration` record with a custom message; does not transfer tokens |
| Reward metadata | `issue_milestone_reward` | Called by anyone | Attaches `reward_details` map to an existing `MilestoneCelebration`; flips `reward_issued = true` |

### Key distinction

- `check_and_distribute_milestone_rewards` — moves real tokens. It is automatic and keyed off `reward_bps`.
- `celebrate_milestone` / `check_and_celebrate_milestones` — create `MilestoneCelebration` records for social/UI purposes. They do **not** transfer tokens and do not consult `reward_bps`.
- `issue_milestone_reward` — annotates a celebration record with off-chain or metadata reward details. It does **not** transfer tokens either.

A milestone can therefore have:
- A token reward (set `reward_bps > 0`) — paid automatically.
- A celebration record (call `check_and_celebrate_milestones`) — separate action.
- Both, neither, or either.

### Celebration event emitted by automatic reward

When `check_and_distribute_milestone_rewards` successfully transfers tokens it emits:

```rust
events::emit_milestone_reached(env, group_id, member, percentage, reward_amount);
```

This is distinct from the `MilestoneCelebration` struct stored by the celebration functions.

---

## Error Handling and Edge Cases

| Situation | Behaviour |
|---|---|
| Pool has fewer tokens than the reward | Transfer skipped; milestone still marked claimed |
| Milestone `reward_bps == 0` | Skipped entirely by `check_and_distribute_milestone_rewards` |
| Same milestone crossed again on later contribution | Bitmask check skips it — reward paid once only |
| Two milestones crossed in one large contribution | Both fire independently in the same call |
| Goal is `Completed`, `Abandoned`, or `Failed` | `contribute_to_savings_goal` rejects the call before reward logic runs |

---

## API Reference

```rust
// Pool management
fn fund_savings_reward_pool(env: Env, admin: Address, amount: i128);
fn get_savings_reward_pool(env: Env) -> i128;

// Goal and milestone setup
fn create_savings_goal(env: Env, member: Address, group_id: u32, name: String,
    description: String, target_amount: i128, token: Address, target_date: u64,
    priority: u32, category: String, metadata: Map<String, String>) -> u32;
fn add_savings_goal_milestones(env: Env, goal_id: u32, milestones: Vec<Milestone>);

// Contributions (triggers automatic reward check)
fn contribute_to_savings_goal(env: Env, goal_id: u32, member: Address,
    amount: i128, source: String) -> GoalContribution;

// Celebration (separate from token rewards)
fn check_and_celebrate_milestones(env: Env, goal_id: u32) -> Vec<MilestoneCelebration>;
fn celebrate_milestone(env: Env, goal_id: u32, milestone_id: u32,
    message: String) -> MilestoneCelebration;
fn issue_milestone_reward(env: Env, celebration_id: u32,
    reward_details: Map<String, String>);

// Bitmask query
fn get_savings_milestones_claimed(env: Env, goal_id: u32, member: Address) -> u64;
```

---

## Testing

Test coverage is in `contracts/ahjoor-rosca/src/test_savings_milestone_rewards.rs`:

| Test | What it verifies |
|---|---|
| `test_milestone_reward_distributed_once` | Reward amount is correct, pool decremented, bitmask set |
| `test_milestone_reward_distributed_exactly_once` | Second contribution does not re-trigger reward |
| `test_reward_pool_depletion_does_not_revert` | Pool exhaustion is graceful; bitmask still set |
| `test_multiple_milestones_crossed_in_one_contribution` | Both milestones pay out in a single call |
| `test_admin_can_fund_reward_pool_post_init` | Pool can be topped up incrementally |
