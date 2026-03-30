#![no_std]
#![allow(deprecated)] // env.events().publish() is deprecated in 25.x but .emit() isn't stable yet

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    Address, Env, String as SorobanString, Symbol,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ADMIN: Symbol = symbol_short!("ADMIN");
const TREASURY: Symbol = symbol_short!("TREASURY");
const FEE_BPS: Symbol = symbol_short!("FEE_BPS");
const NEXT_TRADE_ID: Symbol = symbol_short!("NXTTRD");
const BPS_DIVISOR: i128 = 10_000;

// ---------------------------------------------------------------------------
// TradeStatus
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TradeStatus {
    Created,
    Funded,
    Delivered,
    Completed,
    Disputed,
    Cancelled,
}

// ---------------------------------------------------------------------------
// Trade
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trade {
    pub trade_id: u64,
    pub buyer: Address,
    pub seller: Address,
    pub token: Address,
    pub amount: i128,
    pub status: TradeStatus,
    pub delivered_at: Option<u64>,
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub trade_id: u64,
    pub cid: SorobanString,
    pub submitter: Address,
    pub submitted_at: u64,
}

// ---------------------------------------------------------------------------
// DataKey
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    Trade(u64),
    Evidence(u64),
    Initialized,
    Admin,
    UsdcContract,
    FeeBps,
}

// ---------------------------------------------------------------------------
// Token client — cross-contract calls
// ---------------------------------------------------------------------------

mod token {
    use soroban_sdk::{contractclient, Address, Env};
    #[contractclient(name = "Client")]
    pub trait Token {
        fn transfer(env: Env, from: Address, to: Address, amount: i128);
        fn balance(env: Env, id: Address) -> i128;
    }
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct EscrowContract;

#[contractimpl]
impl EscrowContract {
    pub fn initialize(env: Env, admin: Address, usdc_contract: Address, fee_bps: u32) {
        if env.storage().instance().get::<DataKey, bool>(&DataKey::Initialized).unwrap_or(false) {
            panic!("AlreadyInitialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::UsdcContract, &usdc_contract);
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage().instance().set(&ADMIN, &admin);
        env.storage().instance().set(&FEE_BPS, &fee_bps);
    }

    pub fn create_trade(env: Env, buyer: Address, seller: Address, amount_usdc: i128) -> u64 {
        assert!(amount_usdc > 0, "amount_usdc must be greater than zero");
        buyer.require_auth();

        let next_id: u64 = env.storage().instance().get(&NEXT_TRADE_ID).unwrap_or(1_u64);
        let ledger_seq = env.ledger().sequence() as u64;
        let trade_id = (ledger_seq << 32) | next_id;
        env.storage().instance().set(&NEXT_TRADE_ID, &(next_id + 1));

        let token = env.storage().instance()
            .get::<DataKey, Address>(&DataKey::UsdcContract)
            .unwrap_or_else(|| env.current_contract_address());

        env.storage().persistent().set(
            &DataKey::Trade(trade_id),
            &Trade { trade_id, buyer: buyer.clone(), seller: seller.clone(), token,
                     amount: amount_usdc, status: TradeStatus::Created, delivered_at: None },
        );
        env.events().publish((symbol_short!("TRDCRT"), trade_id), (buyer, seller, amount_usdc));
        trade_id
    }

    pub fn mark_funded(env: Env, trade_id: u64) {
        let key = DataKey::Trade(trade_id);
        let mut trade: Trade = env.storage().persistent().get(&key).expect("TradeNotFound");
        assert!(matches!(trade.status, TradeStatus::Created), "trade must be in Created status");
        trade.buyer.require_auth();
        trade.status = TradeStatus::Funded;
        env.storage().persistent().set(&key, &trade);
    }

    pub fn confirm_delivery(env: Env, trade_id: u64) {
        let key = DataKey::Trade(trade_id);
        let mut trade: Trade = env.storage().persistent().get(&key).expect("TradeNotFound");
        assert!(matches!(trade.status, TradeStatus::Funded), "trade must be funded");
        trade.buyer.require_auth();
        let delivered_at = env.ledger().timestamp();
        trade.status = TradeStatus::Delivered;
        trade.delivered_at = Some(delivered_at);
        env.storage().persistent().set(&key, &trade);
        env.events().publish((symbol_short!("DELCNF"), trade_id), delivered_at);
    }

    /// deliver_item — panics explicitly if trade is Disputed.
    pub fn deliver_item(env: Env, trade_id: u64) {
        let key = DataKey::Trade(trade_id);
        let trade: Trade = env.storage().persistent().get(&key).expect("TradeNotFound");
        assert!(!matches!(trade.status, TradeStatus::Disputed), "cannot deliver while trade is disputed");
        assert!(matches!(trade.status, TradeStatus::Funded), "trade must be funded to deliver");
        let buyer = trade.buyer.clone();
        let delivered_at = env.ledger().timestamp();
        let mut t = trade;
        t.status = TradeStatus::Delivered;
        t.delivered_at = Some(delivered_at);
        buyer.require_auth();
        env.storage().persistent().set(&key, &t);
        env.events().publish((symbol_short!("DELCNF"), trade_id), delivered_at);
    }

    pub fn release_funds(env: Env, trade_id: u64) {
        let key = DataKey::Trade(trade_id);
        let mut trade: Trade = env.storage().persistent().get(&key).expect("TradeNotFound");
        assert!(matches!(trade.status, TradeStatus::Delivered), "trade must be delivered");
        trade.buyer.require_auth();

        let fee_bps: u32 = env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0);
        let fee_amount: i128 = trade.amount * fee_bps as i128 / BPS_DIVISOR;
        let seller_amount: i128 = trade.amount - fee_amount;

        let tok = token::Client::new(&env, &trade.token);
        tok.transfer(&env.current_contract_address(), &trade.seller, &seller_amount);
        if fee_amount > 0 {
            let treasury: Address = env.storage().instance().get(&TREASURY)
                .unwrap_or_else(|| env.current_contract_address());
            tok.transfer(&env.current_contract_address(), &treasury, &fee_amount);
        }
        trade.status = TradeStatus::Completed;
        env.storage().persistent().set(&key, &trade);
        env.events().publish((symbol_short!("RELSD"), trade_id), (seller_amount, fee_amount));
    }

    pub fn submit_evidence(env: Env, trade_id: u64, cid: SorobanString, submitter: Address) {
        submitter.require_auth();
        let trade: Trade = env.storage().persistent().get(&DataKey::Trade(trade_id)).expect("TradeNotFound");
        assert!(submitter == trade.buyer || submitter == trade.seller,
            "Unauthorized: only buyer or seller can submit evidence");
        let ev_key = DataKey::Evidence(trade_id);
        assert!(env.storage().persistent().get::<_, Evidence>(&ev_key).is_none(),
            "EvidenceImmutable: evidence already submitted");
        let submitted_at = env.ledger().timestamp();
        env.storage().persistent().set(&ev_key,
            &Evidence { trade_id, cid: cid.clone(), submitter: submitter.clone(), submitted_at });
        env.events().publish((symbol_short!("EVDSUB"), trade_id), (cid, submitter));
    }

    pub fn get_evidence(env: Env, trade_id: u64) -> Evidence {
        env.storage().persistent().get(&DataKey::Evidence(trade_id)).expect("EvidenceNotFound")
    }

    pub fn initiate_dispute(env: Env, trade_id: u64, initiator: Address) {
        initiator.require_auth();
        let key = DataKey::Trade(trade_id);
        let mut trade: Trade = env.storage().persistent().get(&key).expect("TradeNotFound");
        assert!(initiator == trade.buyer || initiator == trade.seller, "Unauthorized");
        assert!(matches!(trade.status, TradeStatus::Funded | TradeStatus::Delivered),
            "can only dispute a Funded or Delivered trade");
        trade.status = TradeStatus::Disputed;
        env.storage().persistent().set(&key, &trade);
        env.events().publish((symbol_short!("TRDDISP"), trade_id), initiator);
    }

    pub fn resolve_dispute(env: Env, trade_id: u64, seller_gets_bps: u32) {
        assert!(seller_gets_bps <= 10_000, "seller_gets_bps must be <= 10000");
        let admin: Address = env.storage().instance().get(&DataKey::Admin).expect("not initialized");
        admin.require_auth();

        let key = DataKey::Trade(trade_id);
        let mut trade: Trade = env.storage().persistent().get(&key).expect("TradeNotFound");
        assert!(matches!(trade.status, TradeStatus::Disputed), "trade must be in Disputed status");

        let seller_amount: i128 = trade.amount * seller_gets_bps as i128 / BPS_DIVISOR;
        let buyer_amount:  i128 = trade.amount - seller_amount;

        let tok = token::Client::new(&env, &trade.token);
        if seller_amount > 0 { tok.transfer(&env.current_contract_address(), &trade.seller, &seller_amount); }
        if buyer_amount  > 0 { tok.transfer(&env.current_contract_address(), &trade.buyer,  &buyer_amount);  }

        trade.status = TradeStatus::Cancelled;
        env.storage().persistent().set(&key, &trade);
        env.events().publish((symbol_short!("TRDRES"), trade_id), (seller_amount, buyer_amount, seller_gets_bps));
    }

    pub fn get_trade(env: Env, trade_id: u64) -> Trade {
        env.storage().persistent().get(&DataKey::Trade(trade_id)).expect("TradeNotFound")
    }
}

// ---------------------------------------------------------------------------
// Inline unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod unit_tests {
    extern crate std;
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[contract] struct MockToken;
    #[contracttype] #[derive(Clone)] enum MTKey { Balance(Address) }
    #[contractimpl] impl MockToken {
        pub fn mint(env: Env, to: Address, amount: i128) {
            let k = MTKey::Balance(to);
            let c: i128 = env.storage().persistent().get(&k).unwrap_or(0);
            env.storage().persistent().set(&k, &(c + amount));
        }
        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage().persistent().get(&MTKey::Balance(id)).unwrap_or(0)
        }
        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            let fk = MTKey::Balance(from); let tk = MTKey::Balance(to);
            let fb: i128 = env.storage().persistent().get(&fk).unwrap_or(0);
            assert!(fb >= amount, "insufficient balance");
            let tb: i128 = env.storage().persistent().get(&tk).unwrap_or(0);
            env.storage().persistent().set(&fk, &(fb - amount));
            env.storage().persistent().set(&tk, &(tb + amount));
        }
    }

    fn setup() -> (Env, Address, Address, Address, Address, Address) {
        let env = Env::default(); env.mock_all_auths();
        let escrow = env.register(EscrowContract, ());
        let token  = env.register(MockToken, ());
        let admin  = Address::generate(&env);
        let buyer  = Address::generate(&env);
        let seller = Address::generate(&env);
        (env, escrow, token, admin, buyer, seller)
    }

    #[test] fn test_initialize_succeeds() {
        let (env, escrow, token, admin, _, _) = setup();
        EscrowContractClient::new(&env, &escrow).initialize(&admin, &token, &100u32);
        env.as_contract(&escrow, || {
            assert!(env.storage().instance().get::<DataKey, bool>(&DataKey::Initialized).unwrap());
        });
    }

    #[test] #[should_panic(expected = "AlreadyInitialized")]
    fn test_initialize_fails_if_called_twice() {
        let (env, escrow, token, admin, _, _) = setup();
        let c = EscrowContractClient::new(&env, &escrow);
        c.initialize(&admin, &token, &100u32);
        c.initialize(&admin, &token, &100u32);
    }

    #[test] fn test_create_trade_returns_id() {
        let (env, escrow, token, admin, buyer, seller) = setup();
        let c = EscrowContractClient::new(&env, &escrow);
        c.initialize(&admin, &token, &100u32);
        let id = c.create_trade(&buyer, &seller, &10_000);
        assert!(id > 0);
        assert_eq!(c.get_trade(&id).status, TradeStatus::Created);
    }

    #[test] #[should_panic(expected = "amount_usdc must be greater than zero")]
    fn test_create_trade_fails_on_zero_amount() {
        let (env, escrow, token, admin, buyer, seller) = setup();
        let c = EscrowContractClient::new(&env, &escrow);
        c.initialize(&admin, &token, &100u32);
        c.create_trade(&buyer, &seller, &0);
    }

    #[test] fn test_release_funds_correct_split() {
        let (env, escrow, token, admin, buyer, seller) = setup();
        let c = EscrowContractClient::new(&env, &escrow);
        let tok = MockTokenClient::new(&env, &token);
        c.initialize(&admin, &token, &100u32);
        tok.mint(&escrow, &10_000);
        let id = c.create_trade(&buyer, &seller, &10_000);
        c.mark_funded(&id);
        c.confirm_delivery(&id);
        c.release_funds(&id);
        assert_eq!(tok.balance(&seller), 9_900);
    }
}
