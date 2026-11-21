//! Bindings compatibility golden for `get_address` (Issue #328).
//!
//! Produces the ScVal XDR that `simulateTransaction` returns for
//! `get_address` (hit and miss) and compares it byte-for-byte with the
//! checked-in fixture `tests/testdata/bindings/get_address_simulate.v1.json`.
//! `trustbridge-action/src/soroban.ts` consumes that fixture instead of
//! hand-mocking XDR — see `tests/testdata/bindings/README.md`.
//!
//! Regenerate after an intentional return-type change:
//!   UPDATE_GOLDEN=1 cargo test --test bindings_golden
//! and bump `FIXTURE_VERSION` (and the file name) on any breaking change.

use soroban_sdk::{
    testutils::Ledger as _,
    xdr::{Limits, ScVal, WriteXdr},
    Address, Env, IntoVal, String, TryFromVal, Val,
};
use trustbridge_contract::{TrustBridgeContract, TrustBridgeContractClient};

const FIXTURE_VERSION: u32 = 1;
const FIXTURE_PATH: &str = "tests/testdata/bindings/get_address_simulate.v1.json";
const ADMIN: &str = "GAAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQDZ7H";
const USER: &str = "GABAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEJXA";
const REGISTERED_AT: u64 = 1_700_000_000;

fn to_b64(env: &Env, v: Val) -> std::string::String {
    ScVal::try_from_val(env, &v)
        .expect("ScVal conversion")
        .to_xdr_base64(Limits::none())
        .expect("xdr encode")
}

fn render() -> std::string::String {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = REGISTERED_AT);
    let admin = Address::from_str(&env, ADMIN);
    let user = Address::from_str(&env, USER);
    let id = env.register(TrustBridgeContract, ());
    let client = TrustBridgeContractClient::new(&env, &id);
    client.initialize(&admin);
    client.register(
        &String::from_str(&env, "octocat"),
        &user,
        &soroban_sdk::Vec::new(&env),
    );

    let hit = client.get_address(&String::from_str(&env, "octocat"));
    let miss = client.get_address(&String::from_str(&env, "nobody"));
    assert!(hit.is_some() && miss.is_none());
    let hit_xdr = to_b64(&env, hit.clone().into_val(&env));
    let miss_xdr = to_b64(&env, miss.into_val(&env));
    let r = hit.unwrap();
    // First registration: payout defaults to the identity address.
    assert_eq!(r.payout_address, user);

    format!(
        r#"{{
  "fixture": "get_address_simulate",
  "version": {FIXTURE_VERSION},
  "encoding": "base64 XDR of ScVal (simulateTransaction results[0].xdr / retval)",
  "function": "get_address",
  "return_type": "Option<ContributorRecord>",
  "cases": [
    {{
      "name": "registered",
      "args": {{ "github_username": "octocat" }},
      "retval_xdr": "{hit_xdr}",
      "decoded": {{
        "stellar_address": "{USER}",
        "payout_address": "{USER}",
        "registered_at": {at},
        "verified": {verified},
        "is_bot": {is_bot}
      }}
    }},
    {{
      "name": "not_registered",
      "args": {{ "github_username": "nobody" }},
      "retval_xdr": "{miss_xdr}",
      "decoded": null
    }}
  ]
}}
"#,
        at = r.registered_at,
        verified = r.verified,
        is_bot = r.is_bot,
    )
}

#[test]
fn get_address_simulate_golden_matches() {
    let actual = render();
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(FIXTURE_PATH, &actual).expect("write fixture");
        return;
    }
    let expected = std::fs::read_to_string(FIXTURE_PATH)
        .expect("fixture missing — run UPDATE_GOLDEN=1 cargo test --test bindings_golden");
    assert_eq!(
        expected, actual,
        "get_address XDR changed. If intentional: regenerate with UPDATE_GOLDEN=1, \
         bump FIXTURE_VERSION on breaking changes, and notify trustbridge-action."
    );
}
