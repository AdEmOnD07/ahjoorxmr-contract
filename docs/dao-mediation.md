# DAO Mediation

## Overview

The `ahjoor-payments` contract can route an unresolved payment dispute to a configured DAO mediator panel. DAO members vote on whether the merchant or customer should prevail. After the voting window closes, anyone can execute the verdict once the configured minimum number of votes has been cast.

The contract stores each mediation as a `DaoMediationCase` with the payment ID, initiator, timestamps, vote totals, and an execution flag. The case can be inspected with `get_dao_mediation_case`, `get_dao_case_by_payment`, and `get_dao_vote`.

## Configure the DAO

The contract admin calls `configure_dao` with:

- `dao_members`: addresses authorized to vote (up to 20 members).
- `vote_window_seconds`: duration of the voting window after escalation. It must be positive.
- `min_votes`: minimum number of votes required before a verdict can execute. It must be at least 1.

The DAO must be configured before a payment can be escalated. Configuration replaces the current member list and voting parameters.

## When a Payment Can Be Escalated

The payment customer or contract admin calls `escalate_to_dao(initiator, payment_id)`. The call requires the initiator's authorization and succeeds only when all of the following are true:

1. The initiator is the payment customer or contract admin.
2. The payment status is `Disputed` or `EscalatedDispute`.
3. DAO members have been configured.
4. There is no existing open DAO case for the payment.

Escalation creates a case ID, records the current ledger timestamp, and starts the vote window immediately. A payment cannot have two open DAO cases at once.

The admin can use `cancel_dao_escalation(payment_id)` to remove an active case, but only before any DAO member has voted. A new case can then be opened for the payment.

## Voting and Tallying

Each configured DAO member can call `dao_vote(voter, case_id, for_merchant)` once during the vote window. The `for_merchant` value has this meaning:

- `true`: vote for the merchant.
- `false`: vote for the customer.

The contract rejects votes from non-members, duplicate votes, votes on executed cases, and votes submitted after the window closes. Each accepted vote is stored for the member and increments exactly one case counter:

- `votes_for_merchant` for `true` votes.
- `votes_for_customer` for `false` votes.

The total vote count is the sum of both counters. Votes are not weighted, and the number of DAO members who abstain does not count toward `min_votes`.

## Executing the Verdict

After the vote window has elapsed, anyone can call `execute_dao_verdict(case_id)`. Execution is rejected while the window is open, when the total vote count is below `min_votes`, or when the case has already been executed. The payment must also still be in `Disputed` or `EscalatedDispute` status.

The outcome is determined by a strict comparison of the two vote counters:

- **Merchant wins:** when `votes_for_merchant` is greater than `votes_for_customer`. The payment status becomes `Completed`; the contract records that it was not settled by refund.
- **Customer wins:** when `votes_for_customer` is greater than or equal to `votes_for_merchant`. The contract refunds any outstanding escrowed amount to the customer and changes the payment status to `Refunded`.

Therefore, a tie resolves in the customer's favor. Once execution succeeds, the case is marked `executed`, the temporary dispute record is removed, and the corresponding payment-status and DAO-verdict events are emitted.
