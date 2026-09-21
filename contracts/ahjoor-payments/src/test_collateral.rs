#![cfg(test)]
use super::*;
use soroban_sdk::token::Client as TokenClient;
use soroban_sdk::token::StellarAssetClient as TokenAdminClient;
use soroban_sdk::{
    testutils::Address as _,
    Address, Env,
};

// ---------------------------------------------------------------------------
//  Test Helpers
// ---------------------------------------------------------------------------

struct CollateralSetup<'a> {
    env: Env,
    client: AhjoorPaymentsContractClient<'a>,
    /// USDC token (used as collateral token)
    usdc_addr: Address,
    usdc_client: TokenClient<'a>,
    usdc_admin_client: TokenAdminClient<'a>,
    /// A second token for non-USDC payment tests
    other_token_addr: Address,
    other_token_admin_client: TokenAdminClient<'a>,
}

fn collateral_setup<'a>() -> CollateralSetup<'a> {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(AhjoorPaymentsContract, ());
    let client = AhjoorPaymentsContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let usdc_client = TokenClient::new(&env, &usdc_addr);
    let usdc_admin_client = TokenAdminClient::new(&env, &usdc_addr);

    let other_token_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let other_token_admin_client = TokenAdminClient::new(&env, &other_token_addr);

    // Use a dummy oracle address (not called in these tests)
    let oracle_addr = Address::generate(&env);

    client.initialize(&admin, &fee_recipient, &0);
    // Configure USDC as the collateral/settlement token
    client.set_oracle(&oracle_addr, &usdc_addr, &3600);
    // Disable open mode so merchant approval is enforced
    client.set_merchant_open_mode(&false);

    CollateralSetup {
        env,
        client,
        usdc_addr,
        usdc_client,
        usdc_admin_client,
        other_token_addr,
        other_token_admin_client,
    }
}

// ===========================================================================
//  set_min_collateral
// ===========================================================================

#[test]
fn test_set_min_collateral_default() {
    let s = collateral_setup();
    // Default should be DEFAULT_MIN_COLLATERAL (1_000_000)
    assert_eq!(s.client.get_min_collateral(), 1_000_000);
}

#[test]
fn test_set_min_collateral_by_admin() {
    let s = collateral_setup();
    s.client.set_min_collateral(&500_000i128);
    assert_eq!(s.client.get_min_collateral(), 500_000);
}

#[test]
#[should_panic(expected = "min_collateral cannot be negative")]
fn test_set_min_collateral_negative_panics() {
    let s = collateral_setup();
    s.client.set_min_collateral(&-1i128);
}

// ===========================================================================
//  deposit_collateral
// ===========================================================================

#[test]
fn test_deposit_collateral_success() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &5_000_000);

    s.client.deposit_collateral(&merchant, &2_000_000i128);

    assert_eq!(s.client.get_collateral_balance(&merchant), 2_000_000);
    // Tokens moved from merchant to contract
    assert_eq!(s.usdc_client.balance(&merchant), 3_000_000);
    assert_eq!(s.usdc_client.balance(&s.client.address), 2_000_000);
}

#[test]
fn test_deposit_collateral_accumulates() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &5_000_000);

    s.client.deposit_collateral(&merchant, &1_000_000i128);
    s.client.deposit_collateral(&merchant, &500_000i128);

    assert_eq!(s.client.get_collateral_balance(&merchant), 1_500_000);
}

#[test]
#[should_panic(expected = "Deposit amount must be positive")]
fn test_deposit_collateral_zero_panics() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.client.deposit_collateral(&merchant, &0i128);
}

#[test]
#[should_panic(expected = "Deposit amount must be positive")]
fn test_deposit_collateral_negative_panics() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.client.deposit_collateral(&merchant, &-100i128);
}

// ===========================================================================
//  approve_merchant — collateral gate
// ===========================================================================

#[test]
fn test_approve_merchant_requires_collateral() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &5_000_000);

    // Deposit exactly the minimum
    s.client.deposit_collateral(&merchant, &1_000_000i128);
    // Should succeed
    s.client.approve_merchant(&merchant);
    assert!(s.client.is_merchant_approved(&merchant));
}

#[test]
#[should_panic(expected = "Merchant collateral below minimum required")]
fn test_approve_merchant_without_collateral_panics() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    // No collateral deposited
    s.client.approve_merchant(&merchant);
}

#[test]
#[should_panic(expected = "Merchant collateral below minimum required")]
fn test_approve_merchant_insufficient_collateral_panics() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &500_000);

    // Deposit less than minimum (1_000_000)
    s.client.deposit_collateral(&merchant, &500_000i128);
    s.client.approve_merchant(&merchant);
}

// ===========================================================================
//  withdraw_collateral
// ===========================================================================

#[test]
fn test_withdraw_collateral_success() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &5_000_000);

    // Deposit 3x minimum so there is room to withdraw
    s.client.deposit_collateral(&merchant, &3_000_000i128);
    // Withdraw 1x minimum — leaves 2x minimum, still above floor
    s.client.withdraw_collateral(&merchant, &1_000_000i128);

    assert_eq!(s.client.get_collateral_balance(&merchant), 2_000_000);
    assert_eq!(s.usdc_client.balance(&merchant), 3_000_000);
}

#[test]
#[should_panic(expected = "Withdrawal would drop collateral below minimum required")]
fn test_withdraw_collateral_below_minimum_panics() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &5_000_000);

    s.client.deposit_collateral(&merchant, &1_500_000i128);
    // Trying to withdraw 600_000 would leave 900_000 < 1_000_000 minimum
    s.client.withdraw_collateral(&merchant, &600_000i128);
}

#[test]
#[should_panic(expected = "Insufficient collateral balance")]
fn test_withdraw_collateral_exceeds_balance_panics() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &5_000_000);

    s.client.deposit_collateral(&merchant, &2_000_000i128);
    s.client.withdraw_collateral(&merchant, &3_000_000i128);
}

#[test]
#[should_panic(expected = "Withdrawal amount must be positive")]
fn test_withdraw_collateral_zero_panics() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &2_000_000);
    s.client.deposit_collateral(&merchant, &2_000_000i128);
    s.client.withdraw_collateral(&merchant, &0i128);
}

// ===========================================================================
//  resolve_dispute — collateral slashing
// ===========================================================================

fn setup_dispute(s: &CollateralSetup) -> (Address, Address, u32) {
    let merchant = Address::generate(&s.env);
    let customer = Address::generate(&s.env);

    // Merchant deposits collateral and gets approved
    s.usdc_admin_client.mint(&merchant, &5_000_000);
    s.client.deposit_collateral(&merchant, &2_000_000i128);
    s.client.approve_merchant(&merchant);

    // Customer gets USDC and creates a payment
    s.usdc_admin_client.mint(&customer, &1_000_000);
    let payment_id = s.client.create_payment(
        &customer,
        &merchant,
        &500_000i128,
        &s.usdc_addr,
        &None,
        &None,
        &None,
    );

    // Customer disputes the payment
    s.client.dispute_payment(
        &customer,
        &payment_id,
        &soroban_sdk::String::from_str(&s.env, "Item not received"),
    );

    (merchant, customer, payment_id)
}

#[test]
fn test_resolve_dispute_for_customer_slashes_collateral() {
    let s = collateral_setup();
    let (merchant, customer, payment_id) = setup_dispute(&s);

    let collateral_before = s.client.get_collateral_balance(&merchant);
    let customer_balance_before = s.usdc_client.balance(&customer);

    // Resolve in customer's favour
    s.client.resolve_dispute(&payment_id, &false);

    // Customer should have been refunded the payment amount
    assert_eq!(
        s.usdc_client.balance(&customer),
        customer_balance_before + 500_000
    );

    // Collateral should have been slashed by the payment amount
    let collateral_after = s.client.get_collateral_balance(&merchant);
    assert_eq!(collateral_after, collateral_before - 500_000);
}

#[test]
fn test_resolve_dispute_for_merchant_no_slash() {
    let s = collateral_setup();
    let (merchant, _customer, payment_id) = setup_dispute(&s);

    let collateral_before = s.client.get_collateral_balance(&merchant);

    // Resolve in merchant's favour — no slash
    s.client.resolve_dispute(&payment_id, &true);

    assert_eq!(s.client.get_collateral_balance(&merchant), collateral_before);
}

#[test]
fn test_resolve_dispute_slash_capped_by_collateral() {
    let s = collateral_setup();

    // Set minimum collateral to 0 so we can approve with tiny collateral
    s.client.set_min_collateral(&0i128);

    let merchant = Address::generate(&s.env);
    let customer = Address::generate(&s.env);

    // Merchant deposits only 100 (less than the payment amount of 500_000)
    s.usdc_admin_client.mint(&merchant, &100);
    s.client.deposit_collateral(&merchant, &100i128);
    s.client.approve_merchant(&merchant);

    s.usdc_admin_client.mint(&customer, &1_000_000);
    let payment_id = s.client.create_payment(
        &customer,
        &merchant,
        &500_000i128,
        &s.usdc_addr,
        &None,
        &None,
        &None,
    );

    s.client.dispute_payment(
        &customer,
        &payment_id,
        &soroban_sdk::String::from_str(&s.env, "Fraud"),
    );

    s.client.resolve_dispute(&payment_id, &false);

    // Collateral should be fully drained (capped at available balance)
    assert_eq!(s.client.get_collateral_balance(&merchant), 0);
}

#[test]
fn test_resolve_dispute_non_usdc_no_slash() {
    let s = collateral_setup();

    // Set minimum collateral to 0 so we can approve without USDC collateral
    s.client.set_min_collateral(&0i128);

    let merchant = Address::generate(&s.env);
    let customer = Address::generate(&s.env);

    s.client.approve_merchant(&merchant);

    // Mint the other (non-USDC) token to customer
    s.other_token_admin_client.mint(&customer, &1_000_000);

    let payment_id = s.client.create_payment(
        &customer,
        &merchant,
        &500_000i128,
        &s.other_token_addr,
        &None,
        &None,
        &None,
    );

    s.client.dispute_payment(
        &customer,
        &payment_id,
        &soroban_sdk::String::from_str(&s.env, "Wrong item"),
    );

    // No USDC collateral deposited
    let collateral_before = s.client.get_collateral_balance(&merchant);
    s.client.resolve_dispute(&payment_id, &false);

    // Collateral unchanged (non-USDC payment, no slash)
    assert_eq!(
        s.client.get_collateral_balance(&merchant),
        collateral_before
    );
}

// ===========================================================================
//  get_collateral_balance — zero for unknown merchant
// ===========================================================================

#[test]
fn test_get_collateral_balance_unknown_merchant() {
    let s = collateral_setup();
    let unknown = Address::generate(&s.env);
    assert_eq!(s.client.get_collateral_balance(&unknown), 0);
}

// ===========================================================================
//  Additional Collateral Tests
// ===========================================================================

#[test]
fn test_multiple_merchants_independent_collateral() {
    let s = collateral_setup();
    let merchant1 = Address::generate(&s.env);
    let merchant2 = Address::generate(&s.env);
    let merchant3 = Address::generate(&s.env);

    s.usdc_admin_client.mint(&merchant1, &5_000_000);
    s.usdc_admin_client.mint(&merchant2, &3_000_000);
    s.usdc_admin_client.mint(&merchant3, &2_000_000);

    // Each merchant deposits different amounts
    s.client.deposit_collateral(&merchant1, &2_000_000i128);
    s.client.deposit_collateral(&merchant2, &1_500_000i128);
    s.client.deposit_collateral(&merchant3, &1_000_000i128);

    // Verify each merchant has independent collateral
    assert_eq!(s.client.get_collateral_balance(&merchant1), 2_000_000);
    assert_eq!(s.client.get_collateral_balance(&merchant2), 1_500_000);
    assert_eq!(s.client.get_collateral_balance(&merchant3), 1_000_000);

    // Withdraw from one merchant shouldn't affect others
    s.client.withdraw_collateral(&merchant1, &500_000i128);
    assert_eq!(s.client.get_collateral_balance(&merchant1), 1_500_000);
    assert_eq!(s.client.get_collateral_balance(&merchant2), 1_500_000);
    assert_eq!(s.client.get_collateral_balance(&merchant3), 1_000_000);
}

#[test]
fn test_withdraw_exact_balance_above_minimum() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &5_000_000);

    // Deposit 2x minimum
    s.client.deposit_collateral(&merchant, &2_000_000i128);
    
    // Withdraw exactly 1x minimum, leaving 1x minimum (boundary case)
    s.client.withdraw_collateral(&merchant, &1_000_000i128);

    assert_eq!(s.client.get_collateral_balance(&merchant), 1_000_000);
    assert_eq!(s.usdc_client.balance(&merchant), 4_000_000);
}

#[test]
#[should_panic(expected = "Withdrawal would drop collateral below minimum required")]
fn test_withdraw_one_token_below_minimum_panics() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &5_000_000);

    s.client.deposit_collateral(&merchant, &2_000_000i128);
    
    // Try to withdraw 1_000_001, leaving 999_999 which is below minimum
    s.client.withdraw_collateral(&merchant, &1_000_001i128);
}

#[test]
fn test_admin_can_change_min_collateral_multiple_times() {
    let s = collateral_setup();
    
    // Change minimum several times
    s.client.set_min_collateral(&500_000i128);
    assert_eq!(s.client.get_min_collateral(), 500_000);

    s.client.set_min_collateral(&2_000_000i128);
    assert_eq!(s.client.get_min_collateral(), 2_000_000);

    s.client.set_min_collateral(&750_000i128);
    assert_eq!(s.client.get_min_collateral(), 750_000);

    s.client.set_min_collateral(&0i128);
    assert_eq!(s.client.get_min_collateral(), 0);
}

#[test]
fn test_merchant_with_pending_collateral_can_deposit_more() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &10_000_000);

    // Initial deposit
    s.client.deposit_collateral(&merchant, &1_000_000i128);
    
    // Merchant decides to add more collateral later
    s.client.deposit_collateral(&merchant, &2_000_000i128);
    s.client.deposit_collateral(&merchant, &500_000i128);

    assert_eq!(s.client.get_collateral_balance(&merchant), 3_500_000);
}

#[test]
fn test_multiple_disputes_slash_collateral_progressively() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    let customer1 = Address::generate(&s.env);
    let customer2 = Address::generate(&s.env);

    // Merchant deposits substantial collateral
    s.usdc_admin_client.mint(&merchant, &10_000_000);
    s.client.deposit_collateral(&merchant, &5_000_000i128);
    s.client.approve_merchant(&merchant);

    // First payment and dispute
    s.usdc_admin_client.mint(&customer1, &1_000_000);
    let payment_id1 = s.client.create_payment(
        &customer1,
        &merchant,
        &300_000i128,
        &s.usdc_addr,
        &None,
        &None,
        &None,
    );
    s.client.dispute_payment(
        &customer1,
        &payment_id1,
        &soroban_sdk::String::from_str(&s.env, "Issue 1"),
    );
    s.client.resolve_dispute(&payment_id1, &false);
    
    assert_eq!(s.client.get_collateral_balance(&merchant), 4_700_000);

    // Second payment and dispute
    s.usdc_admin_client.mint(&customer2, &1_000_000);
    let payment_id2 = s.client.create_payment(
        &customer2,
        &merchant,
        &500_000i128,
        &s.usdc_addr,
        &None,
        &None,
        &None,
    );
    s.client.dispute_payment(
        &customer2,
        &payment_id2,
        &soroban_sdk::String::from_str(&s.env, "Issue 2"),
    );
    s.client.resolve_dispute(&payment_id2, &false);

    // Collateral slashed twice
    assert_eq!(s.client.get_collateral_balance(&merchant), 4_200_000);
}

#[test]
fn test_merchant_can_top_up_collateral_after_slash() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    let customer = Address::generate(&s.env);

    s.usdc_admin_client.mint(&merchant, &10_000_000);
    s.client.deposit_collateral(&merchant, &3_000_000i128);
    s.client.approve_merchant(&merchant);

    // Create and resolve dispute (slash collateral)
    s.usdc_admin_client.mint(&customer, &1_000_000);
    let payment_id = s.client.create_payment(
        &customer,
        &merchant,
        &500_000i128,
        &s.usdc_addr,
        &None,
        &None,
        &None,
    );
    s.client.dispute_payment(
        &customer,
        &payment_id,
        &soroban_sdk::String::from_str(&s.env, "Problem"),
    );
    s.client.resolve_dispute(&payment_id, &false);

    assert_eq!(s.client.get_collateral_balance(&merchant), 2_500_000);

    // Merchant tops up collateral
    s.client.deposit_collateral(&merchant, &1_000_000i128);
    assert_eq!(s.client.get_collateral_balance(&merchant), 3_500_000);
}

#[test]
fn test_zero_collateral_minimum_allows_approval_without_deposit() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);

    // Set minimum to zero
    s.client.set_min_collateral(&0i128);

    // Should be able to approve without any collateral
    s.client.approve_merchant(&merchant);
    assert!(s.client.is_merchant_approved(&merchant));
    assert_eq!(s.client.get_collateral_balance(&merchant), 0);
}

#[test]
fn test_large_collateral_deposit() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    
    // Test with a very large amount (100 million)
    let large_amount = 100_000_000i128;
    s.usdc_admin_client.mint(&merchant, &large_amount);

    s.client.deposit_collateral(&merchant, &large_amount);
    assert_eq!(s.client.get_collateral_balance(&merchant), large_amount);
    assert_eq!(s.usdc_client.balance(&merchant), 0);
}

#[test]
fn test_collateral_state_persists_across_operations() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &10_000_000);

    // Deposit
    s.client.deposit_collateral(&merchant, &3_000_000i128);
    let balance_after_deposit = s.client.get_collateral_balance(&merchant);
    assert_eq!(balance_after_deposit, 3_000_000);

    // Approve merchant
    s.client.approve_merchant(&merchant);
    let balance_after_approval = s.client.get_collateral_balance(&merchant);
    assert_eq!(balance_after_approval, 3_000_000);

    // Withdraw some
    s.client.withdraw_collateral(&merchant, &500_000i128);
    let balance_after_withdrawal = s.client.get_collateral_balance(&merchant);
    assert_eq!(balance_after_withdrawal, 2_500_000);

    // Deposit more
    s.client.deposit_collateral(&merchant, &1_000_000i128);
    let final_balance = s.client.get_collateral_balance(&merchant);
    assert_eq!(final_balance, 3_500_000);
}

#[test]
#[should_panic(expected = "Merchant collateral below minimum required")]
fn test_raising_min_collateral_affects_new_approvals() {
    let s = collateral_setup();
    let merchant = Address::generate(&s.env);
    s.usdc_admin_client.mint(&merchant, &5_000_000);

    // Deposit 1.5M with minimum at 1M
    s.client.deposit_collateral(&merchant, &1_500_000i128);

    // Admin raises minimum to 2M
    s.client.set_min_collateral(&2_000_000i128);

    // Attempt to approve with insufficient collateral (under new minimum)
    s.client.approve_merchant(&merchant);
}
