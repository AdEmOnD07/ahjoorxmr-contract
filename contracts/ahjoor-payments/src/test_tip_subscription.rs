#![cfg(test)]
use super::*;
use soroban_sdk::token::Client as TokenClient;
use soroban_sdk::token::StellarAssetClient as TokenAdminClient;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

struct TipSubSetup<'a> {
    env: Env,
    client: AhjoorPaymentsContractClient<'a>,
    token_addr: Address,
    token_client: TokenClient<'a>,
    token_admin: TokenAdminClient<'a>,
}

fn tip_sub_setup<'a>() -> TipSubSetup<'a> {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let contract_id = env.register(AhjoorPaymentsContract, ());
    let client = AhjoorPaymentsContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    let token_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_client = TokenClient::new(&env, &token_addr);
    let token_admin = TokenAdminClient::new(&env, &token_addr);

    client.initialize(&admin, &fee_recipient, &0);

    TipSubSetup {
        env,
        client,
        token_addr,
        token_client,
        token_admin,
    }
}

fn approve(s: &TipSubSetup, customer: &Address, amount: i128) {
    s.token_client.approve(
        customer,
        &s.client.address,
        &amount,
        &(s.env.ledger().sequence() + 10_000),
    );
}

#[test]
fn test_tip_interval_enforcement() {
    let s = tip_sub_setup();
    let customer = Address::generate(&s.env);
    let recipient = Address::generate(&s.env);
    s.token_admin.mint(&customer, &1_000);
    approve(&s, &customer, 1_000);

    let sub_id = s.client.create_tip_subscription(
        &customer,
        &recipient,
        &s.token_addr,
        &10,
        &50,
    );

    // First execution is due immediately
    s.client.execute_due_tip(&sub_id);
    assert_eq!(s.token_client.balance(&recipient), 10);
    assert_eq!(s.client.get_tip_subscription(&sub_id).executions, 1);

    // Second execution before interval elapses must fail
    let early = s.client.try_execute_due_tip(&sub_id);
    assert!(early.is_err());

    let due = s.client.get_tip_subscription(&sub_id).next_due_ledger;
    s.env.ledger().set_sequence_number(due);
    s.client.execute_due_tip(&sub_id);
    assert_eq!(s.token_client.balance(&recipient), 20);
    assert_eq!(s.client.get_tip_subscription(&sub_id).executions, 2);
}

#[test]
fn test_tip_cancellation_stops_execution() {
    let s = tip_sub_setup();
    let customer = Address::generate(&s.env);
    let recipient = Address::generate(&s.env);
    s.token_admin.mint(&customer, &1_000);
    approve(&s, &customer, 1_000);

    let sub_id = s.client.create_tip_subscription(
        &customer,
        &recipient,
        &s.token_addr,
        &5,
        &10,
    );

    s.client.cancel_tip_subscription(&customer, &sub_id);
    assert!(!s.client.get_tip_subscription(&sub_id).active);

    let result = s.client.try_execute_due_tip(&sub_id);
    assert!(result.is_err());
}

#[test]
fn test_tip_execution_by_third_party_relayer() {
    let s = tip_sub_setup();
    let customer = Address::generate(&s.env);
    let recipient = Address::generate(&s.env);
    let _relayer = Address::generate(&s.env);
    s.token_admin.mint(&customer, &1_000);
    approve(&s, &customer, 1_000);

    let sub_id = s.client.create_tip_subscription(
        &customer,
        &recipient,
        &s.token_addr,
        &15,
        &20,
    );

    // Anyone can execute once due (no caller auth required)
    s.client.execute_due_tip(&sub_id);
    assert_eq!(s.token_client.balance(&recipient), 15);

    let due = s.client.get_tip_subscription(&sub_id).next_due_ledger;
    s.env.ledger().set_sequence_number(due);
    s.client.execute_due_tip(&sub_id);
    assert_eq!(s.token_client.balance(&recipient), 30);
}

#[test]
fn test_tip_subscription_independent_of_invoice_model() {
    let s = tip_sub_setup();
    let customer = Address::generate(&s.env);
    let recipient = Address::generate(&s.env);
    s.token_admin.mint(&customer, &500);
    approve(&s, &customer, 500);

    let sub_id = s.client.create_tip_subscription(
        &customer,
        &recipient,
        &s.token_addr,
        &7,
        &5,
    );

    let sub = s.client.get_tip_subscription(&sub_id);
    assert_eq!(sub.amount, 7);
    assert_eq!(sub.customer, customer);
    assert_eq!(sub.recipient, recipient);
    assert!(sub.active);
    // Tip subscriptions are stored separately — payment counter unaffected
    assert_eq!(s.client.get_payment_counter(), 0);
}
