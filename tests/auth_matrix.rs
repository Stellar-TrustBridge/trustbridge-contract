//! CSV-driven authorization matrix (Issue #579).
//!
//! `tests/auth_matrix.csv` is the single source of truth for who may call each
//! public entrypoint. This driver reads it at compile time and, for every row,
//! deploys a fresh fixture, sets up the operation's preconditions, then invokes
//! the entrypoint with **only** the row's actor signing (`env.mock_auths`, not
//! `mock_all_auths`).
//!
//! # CSV columns
//!
//! | Column           | Meaning                                                    |
//! |------------------|------------------------------------------------------------|
//! | `operation`      | Contract entrypoint name, exactly as exported.             |
//! | `actor`          | `admin`, `second_admin`, `guardian`, `upgrader`, `verifier`, `revoker`, `user` (registrant of `octocat`), `random`, or `none` (no signer). |
//! | `expected_error` | Empty: the call must return `Ok`. Otherwise a `ContractError` variant name. |
//!
//! `NotAuthorized` also matches a host auth failure for entrypoints that call
//! `require_auth` on a fixed identity (e.g. the stored admin) rather than on a
//! `caller` argument. The driver confirms such a failure is auth-related by
//! replaying the row with every auth mocked and requiring that the host error
//! disappears.
//!
//! Every entrypoint whose body calls `require_auth` must have at least one
//! row (`every_auth_gated_entrypoint_has_a_matrix_row`); a new entrypoint
//! therefore needs a CSV row and a dispatcher arm in [`plan`] to pass.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    vec, Address, Bytes, BytesN, Env, IntoVal, String, Symbol, Val, Vec,
};
use trustbridge_contract::{ContractError, OracleProof, Role, TrustBridgeContract};

const MATRIX: &str = include_str!("auth_matrix.csv");
/// Every file holding a `#[contractimpl]` block on `TrustBridgeContract`
/// (split out of `lib.rs` in Issue #601).
const CONTRACT_SRCS: &[&str] = &[
    include_str!("../src/contract/admin.rs"),
    include_str!("../src/contract/roles.rs"),
    include_str!("../src/contract/governance.rs"),
    include_str!("../src/contract/config.rs"),
    include_str!("../src/contract/registry.rs"),
    include_str!("../src/contract/verify.rs"),
    include_str!("../src/contract/rotation.rs"),
    include_str!("../src/contract/health.rs"),
    include_str!("../src/contract/challenge.rs"),
    include_str!("../src/contract/maintenance.rs"),
];

/// Fixed registrant (contract-type, so `mock_auths` can sign for it) that the
/// oracle proof fixture below binds to.
const USER_STRKEY: &str = "CAZOJB42QBRQC44KFDMPWIKO5JBARJ3MBALJQU2T7M2WZUDZ2D3FUXZR";
/// Ed25519 key with seed `sha256("trustbridge-oracle-test-key-v1")` (same as
/// `src/oracle_proof.rs` tests).
const ORACLE_PUBKEY: [u8; 32] = [
    179, 58, 29, 50, 177, 220, 29, 101, 34, 2, 132, 169, 155, 83, 4, 194, 225, 186, 125, 165, 137,
    197, 121, 222, 196, 246, 147, 247, 102, 8, 90, 166,
];
/// Signature over `octocat:<USER_STRKEY>:0`.
const ORACLE_SIG: [u8; 64] = [
    119, 66, 42, 126, 119, 125, 225, 20, 230, 191, 213, 77, 251, 218, 203, 34, 10, 157, 179, 220,
    98, 214, 153, 200, 200, 174, 50, 27, 149, 203, 145, 63, 66, 235, 102, 208, 199, 126, 244, 182,
    206, 237, 117, 9, 78, 162, 99, 94, 177, 163, 60, 90, 109, 100, 224, 66, 183, 239, 92, 104, 59,
    116, 147, 0,
];

struct Row {
    line: usize,
    operation: std::string::String,
    actor: std::string::String,
    expected: Option<std::string::String>,
}

fn rows() -> std::vec::Vec<Row> {
    let mut lines = MATRIX.lines().enumerate();
    let (_, header) = lines.next().expect("auth_matrix.csv is empty");
    assert_eq!(header.trim(), "operation,actor,expected_error");
    lines
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            let cols: std::vec::Vec<&str> = l.split(',').map(str::trim).collect();
            assert_eq!(
                cols.len(),
                3,
                "auth_matrix.csv line {}: expected 3 columns",
                i + 1
            );
            Row {
                line: i + 1,
                operation: cols[0].into(),
                actor: cols[1].into(),
                expected: (!cols[2].is_empty()).then(|| cols[2].into()),
            }
        })
        .collect()
}

struct Fixture {
    env: Env,
    contract_id: Address,
    admin: Address,
    second_admin: Address,
    guardian: Address,
    upgrader: Address,
    verifier: Address,
    revoker: Address,
    user: Address,
    random: Address,
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let contract_id = env.register(TrustBridgeContract, ());
        let fx = Fixture {
            admin: Address::generate(&env),
            second_admin: Address::generate(&env),
            guardian: Address::generate(&env),
            upgrader: Address::generate(&env),
            verifier: Address::generate(&env),
            revoker: Address::generate(&env),
            user: Address::from_str(&env, USER_STRKEY),
            random: Address::generate(&env),
            contract_id,
            env,
        };
        fx.setup(|env| {
            TrustBridgeContract::initialize(env.clone(), fx.admin.clone()).unwrap();
        });
        fx.setup(|env| {
            TrustBridgeContract::set_guardian(env.clone(), fx.guardian.clone()).unwrap()
        });
        for (who, role) in [
            (&fx.second_admin, Role::Admin),
            (&fx.upgrader, Role::Upgrader),
            (&fx.verifier, Role::Verifier),
            (&fx.revoker, Role::Revoker),
        ] {
            fx.setup(|env| TrustBridgeContract::set_role(env.clone(), who.clone(), role).unwrap());
        }
        fx.register("octocat", &fx.user);
        fx
    }

    /// Runs a precondition step with every auth mocked.
    fn setup<T>(&self, f: impl FnOnce(&Env) -> T) -> T {
        self.env.mock_all_auths();
        self.env.as_contract(&self.contract_id, || f(&self.env))
    }

    fn s(&self, v: &str) -> String {
        String::from_str(&self.env, v)
    }

    fn names(&self, v: &[&str]) -> Vec<String> {
        let mut out = Vec::new(&self.env);
        for n in v {
            out.push_back(self.s(n));
        }
        out
    }

    fn register(&self, name: &str, owner: &Address) {
        self.setup(|env| {
            TrustBridgeContract::register(env.clone(), self.s(name), owner.clone(), Vec::new(env))
                .unwrap()
        });
    }

    fn actor(&self, name: &str) -> Option<Address> {
        Some(match name {
            "admin" => self.admin.clone(),
            "second_admin" => self.second_admin.clone(),
            "guardian" => self.guardian.clone(),
            "upgrader" => self.upgrader.clone(),
            "verifier" => self.verifier.clone(),
            "revoker" => self.revoker.clone(),
            "user" => self.user.clone(),
            "random" => self.random.clone(),
            "none" => return None,
            other => panic!("unknown actor `{other}`"),
        })
    }

    fn hash(&self, b: u8) -> BytesN<32> {
        BytesN::from_array(&self.env, &[b; 32])
    }

    /// Uploads a minimal valid contract WASM so upgrade paths have a real
    /// executable to switch to.
    #[allow(deprecated)] // `protocol_version` is the only way to match the host.
    fn uploaded_wasm(&self) -> BytesN<32> {
        use soroban_sdk::xdr::{Limits, ScEnvMetaEntry, ScEnvMetaEntryInterfaceVersion, WriteXdr};
        let meta = ScEnvMetaEntry::ScEnvMetaKindInterfaceVersion(ScEnvMetaEntryInterfaceVersion {
            protocol: self.env.ledger().protocol_version(),
            pre_release: 0,
        })
        .to_xdr(Limits::none())
        .unwrap();
        let name = b"contractenvmetav0";
        let mut section = std::vec![name.len() as u8];
        section.extend_from_slice(name);
        section.extend_from_slice(&meta);
        let mut wasm = std::vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x00];
        assert!(
            section.len() < 0x80,
            "custom section must fit a 1-byte LEB128 size"
        );
        wasm.push(section.len() as u8);
        wasm.extend_from_slice(&section);
        self.env
            .deployer()
            .upload_contract_wasm(Bytes::from_slice(&self.env, &wasm))
    }
}

/// One planned invocation: the entrypoint, its arguments, an optional target
/// contract (defaults to the fixture), and any non-role co-signers the
/// entrypoint also requires (e.g. a sponsor or a rotation's new address).
struct Call {
    func: std::string::String,
    args: Vec<Val>,
    contract: Option<Address>,
    co_signers: std::vec::Vec<Address>,
}

fn call(func: &str, args: Vec<Val>) -> Call {
    Call {
        func: func.into(),
        args,
        contract: None,
        co_signers: std::vec::Vec::new(),
    }
}

/// Sets up `op`'s preconditions on `fx` and returns the invocation to test.
/// `caller` is the row's actor, passed wherever the entrypoint takes one.
#[allow(clippy::too_many_lines)]
fn plan(fx: &Fixture, op: &str, caller: &Address) -> Call {
    let env = &fx.env;
    let c = caller.clone();
    let octocat = fx.s("octocat");
    match op {
        "initialize" => {
            let fresh = env.register(TrustBridgeContract, ());
            Call {
                contract: Some(fresh),
                ..call(op, vec![env, fx.admin.into_val(env)])
            }
        }
        "register" => call(
            "register",
            vec![
                env,
                fx.s("newcomer").into_val(env),
                c.into_val(env),
                Vec::<Address>::new(env).into_val(env),
            ],
        ),
        "register_sponsored" => {
            let sponsor = Address::generate(env);
            Call {
                co_signers: std::vec![sponsor.clone()],
                ..call(
                    "register_sponsored",
                    vec![
                        env,
                        fx.s("newcomer").into_val(env),
                        c.into_val(env),
                        sponsor.into_val(env),
                    ],
                )
            }
        }
        "pause" => call("pause", vec![env, 1u32.into_val(env)]),
        "set_paused" => call(
            "set_paused",
            vec![env, true.into_val(env), 1u32.into_val(env)],
        ),
        "unpause" => {
            fx.setup(|e| TrustBridgeContract::pause(e.clone(), 1).unwrap());
            call("unpause", vec![env, 4u32.into_val(env)])
        }
        "emergency_pause" => call("emergency_pause", vec![env, c.into_val(env)]),
        "clear_emergency_pause" => {
            fx.setup(|e| {
                TrustBridgeContract::emergency_pause(e.clone(), fx.admin.clone()).unwrap()
            });
            call("clear_emergency_pause", vec![env])
        }
        "set_guardian" => call(
            "set_guardian",
            vec![env, Address::generate(env).into_val(env)],
        ),
        "remove_guardian" => call("remove_guardian", vec![env]),
        "set_role" => call(
            "set_role",
            vec![
                env,
                Address::generate(env).into_val(env),
                Role::Verifier.into_val(env),
            ],
        ),
        "set_role_with_expiry" => call(
            "set_role_with_expiry",
            vec![
                env,
                Address::generate(env).into_val(env),
                Role::Verifier.into_val(env),
                Some(10_000u64).into_val(env),
            ],
        ),
        "remove_role" => call("remove_role", vec![env, fx.verifier.into_val(env)]),
        "set_role_delay" => call("set_role_delay", vec![env, 60u64.into_val(env)]),
        "activate_role" | "cancel_role_grant" => {
            // Timelocked grant: queue it, then advance past the delay.
            let target = Address::generate(env);
            fx.setup(|e| TrustBridgeContract::set_role_delay(e.clone(), 60).unwrap());
            fx.setup(|e| {
                TrustBridgeContract::set_role(e.clone(), target.clone(), Role::Verifier).unwrap()
            });
            env.ledger().set_timestamp(env.ledger().timestamp() + 61);
            call(op, vec![env, target.into_val(env)])
        }
        "add_verifier" => call(
            "add_verifier",
            vec![
                env,
                Address::generate(env).into_val(env),
                10_000u64.into_val(env),
            ],
        ),
        "remove_verifier" => {
            let v = Address::generate(env);
            fx.setup(|e| TrustBridgeContract::add_verifier(e.clone(), v.clone(), 10_000).unwrap());
            call("remove_verifier", vec![env, v.into_val(env)])
        }
        "set_verify_limit" => call("set_verify_limit", vec![env, 5u32.into_val(env)]),
        "adopt_network_tag" => call("adopt_network_tag", vec![env]),
        "set_cooldown" => call("set_cooldown", vec![env, 60u64.into_val(env)]),
        "attest_upgrade" => call(
            "attest_upgrade",
            vec![env, fx.hash(1).into_val(env), 10_000u64.into_val(env)],
        ),
        "clear_attestation" => {
            fx.setup(|e| {
                TrustBridgeContract::attest_upgrade(e.clone(), fx.hash(1), 10_000).unwrap()
            });
            call("clear_attestation", vec![env])
        }
        "set_provenance_digests" => {
            // Digests annotate the provenance an upgrade writes.
            let wasm = fx.uploaded_wasm();
            fx.setup(|e| TrustBridgeContract::upgrade(e.clone(), wasm).unwrap());
            // Swap the native test contract back in; storage is kept.
            env.register_at(&fx.contract_id, TrustBridgeContract, ());
            call(
                "set_provenance_digests",
                vec![
                    env,
                    Some(fx.hash(2)).into_val(env),
                    Some(fx.hash(3)).into_val(env),
                ],
            )
        }
        "upgrade" => call("upgrade", vec![env, fx.uploaded_wasm().into_val(env)]),
        "set_upgrade_threshold" => call(
            "set_upgrade_threshold",
            vec![env, c.into_val(env), 1u32.into_val(env)],
        ),
        "propose_multisig_upgrade" => call(
            "propose_multisig_upgrade",
            vec![
                env,
                c.into_val(env),
                fx.hash(1).into_val(env),
                0u64.into_val(env),
            ],
        ),
        "approve_upgrade" | "execute_upgrade" | "cancel_upgrade_proposal" => {
            // For approve, the proposer must differ from every tested actor so
            // the row exercises auth rather than duplicate-approval rejection.
            let proposer = if op == "approve_upgrade" {
                let p = Address::generate(env);
                fx.setup(|e| {
                    TrustBridgeContract::set_role(e.clone(), p.clone(), Role::Upgrader).unwrap()
                });
                p
            } else {
                fx.admin.clone()
            };
            let wasm = fx.uploaded_wasm();
            fx.setup(|e| {
                TrustBridgeContract::propose_multisig_upgrade(
                    e.clone(),
                    proposer.clone(),
                    wasm.clone(),
                    0,
                )
                .unwrap()
            });
            let id = fx
                .setup(|e| TrustBridgeContract::get_upgrade_proposal(e.clone()))
                .unwrap()
                .id;
            call(op, vec![env, c.into_val(env), id.into_val(env)])
        }
        "stage_wasm" => call(
            "stage_wasm",
            vec![env, c.into_val(env), fx.hash(1).into_val(env)],
        ),
        "clear_staged" => {
            fx.setup(|e| {
                TrustBridgeContract::stage_wasm(e.clone(), fx.admin.clone(), fx.hash(1)).unwrap()
            });
            call("clear_staged", vec![env, c.into_val(env)])
        }
        "config_verification" => call(
            "config_verification",
            vec![
                env,
                c.into_val(env),
                Symbol::new(env, "github").into_val(env),
                0u64.into_val(env),
                1u32.into_val(env),
            ],
        ),
        "migrate" => call("migrate", vec![env, (99u32, 0u32, 0u32).into_val(env)]),
        "extend_registry_ttl" => call(
            "extend_registry_ttl",
            vec![env, fx.names(&["octocat"]).into_val(env)],
        ),
        "batch_remove" => call(
            "batch_remove",
            vec![env, c.into_val(env), fx.names(&["octocat"]).into_val(env)],
        ),
        "remove" => call("remove", vec![env, c.into_val(env), octocat.into_val(env)]),
        "verify" => call("verify", vec![env, c.into_val(env), octocat.into_val(env)]),
        "batch_verify" => call(
            "batch_verify",
            vec![env, c.into_val(env), fx.names(&["octocat"]).into_val(env)],
        ),
        "revoke_verification" => {
            fx.setup(|e| {
                TrustBridgeContract::verify(e.clone(), fx.admin.clone(), octocat.clone()).unwrap()
            });
            call(
                "revoke_verification",
                vec![
                    env,
                    c.into_val(env),
                    octocat.into_val(env),
                    1u32.into_val(env),
                ],
            )
        }
        "set_bot_status" => call(
            "set_bot_status",
            vec![
                env,
                c.into_val(env),
                octocat.into_val(env),
                true.into_val(env),
            ],
        ),
        "rename" => call(
            "rename",
            vec![
                env,
                c.into_val(env),
                octocat.into_val(env),
                fx.s("octocat2").into_val(env),
            ],
        ),
        "set_rotation_delay" => call("set_rotation_delay", vec![env, 100u64.into_val(env)]),
        "request_address_rotation" => {
            let new_address = Address::generate(env);
            Call {
                co_signers: std::vec![new_address.clone()],
                ..call(
                    "request_address_rotation",
                    vec![env, octocat.into_val(env), new_address.into_val(env)],
                )
            }
        }
        "execute_address_rotation" => {
            // `alice` rotates onto the actor's key; only the new key signs.
            let old = Address::generate(env);
            fx.register("alice", &old);
            fx.setup(|e| {
                TrustBridgeContract::request_address_rotation(e.clone(), fx.s("alice"), c.clone())
                    .unwrap()
            });
            call(
                "execute_address_rotation",
                vec![env, fx.s("alice").into_val(env)],
            )
        }
        "cancel_address_rotation" => {
            let new_address = Address::generate(env);
            fx.setup(|e| {
                TrustBridgeContract::request_address_rotation(
                    e.clone(),
                    octocat.clone(),
                    new_address.clone(),
                )
                .unwrap()
            });
            call(
                "cancel_address_rotation",
                vec![env, c.into_val(env), octocat.into_val(env)],
            )
        }
        "start_challenge" => call(
            "start_challenge",
            vec![env, c.into_val(env), octocat.into_val(env)],
        ),
        "cancel_challenge" | "complete_challenge" => {
            fx.setup(|e| {
                TrustBridgeContract::start_challenge(e.clone(), fx.admin.clone(), octocat.clone())
                    .unwrap()
            });
            if op == "complete_challenge" {
                env.ledger()
                    .set_timestamp(env.ledger().timestamp() + 172_801);
            }
            call(op, vec![env, c.into_val(env), octocat.into_val(env)])
        }
        "add_reserved" => call("add_reserved", vec![env, fx.s("reserved").into_val(env)]),
        "remove_reserved" => {
            fx.setup(|e| TrustBridgeContract::add_reserved(e.clone(), fx.s("reserved")).unwrap());
            call("remove_reserved", vec![env, fx.s("reserved").into_val(env)])
        }
        "get_reserved_list" => call("get_reserved_list", vec![env]),
        "compact_index" => call("compact_index", vec![env]),
        "repair_index" => call("repair_index", vec![env, false.into_val(env)]),
        "get_registered_page" => call(
            "get_registered_page",
            vec![env, 0u32.into_val(env), 10u32.into_val(env)],
        ),
        "get_all_registered" => call("get_all_registered", vec![env]),
        "get_registered_paginated" | "export_attestation" => call(
            op,
            vec![
                env,
                Option::<BytesN<8>>::None.into_val(env),
                10u32.into_val(env),
            ],
        ),
        "set_batch_remove_threshold" => {
            call("set_batch_remove_threshold", vec![env, 3u32.into_val(env)])
        }
        "get_batch_remove_threshold" => call("get_batch_remove_threshold", vec![env]),
        "propose_batch_remove" => {
            fx.setup(|e| TrustBridgeContract::set_batch_remove_threshold(e.clone(), 1).unwrap());
            call(
                "propose_batch_remove",
                vec![env, c.into_val(env), fx.names(&["octocat"]).into_val(env)],
            )
        }
        "execute_batch_remove" | "cancel_batch_remove" | "get_pending_batch_remove" => {
            fx.setup(|e| TrustBridgeContract::set_batch_remove_threshold(e.clone(), 1).unwrap());
            fx.setup(|e| {
                TrustBridgeContract::propose_batch_remove(
                    e.clone(),
                    fx.admin.clone(),
                    fx.names(&["octocat"]),
                )
                .unwrap()
            });
            if op == "get_pending_batch_remove" {
                call(op, vec![env])
            } else {
                call(op, vec![env, c.into_val(env)])
            }
        }
        "set_oracle_allowlist" => {
            let mut keys = Vec::new(env);
            keys.push_back(BytesN::from_array(env, &ORACLE_PUBKEY));
            call("set_oracle_allowlist", vec![env, keys.into_val(env)])
        }
        "verify_with_oracle_proof" => {
            let mut keys = Vec::new(env);
            keys.push_back(BytesN::from_array(env, &ORACLE_PUBKEY));
            fx.setup(|e| TrustBridgeContract::set_oracle_allowlist(e.clone(), keys).unwrap());
            let message = std::format!("octocat:{USER_STRKEY}:0");
            let proof = OracleProof {
                oracle_pubkey: BytesN::from_array(env, &ORACLE_PUBKEY),
                message: Bytes::from_slice(env, message.as_bytes()),
                signature: BytesN::from_array(env, &ORACLE_SIG),
                expires_at: 0,
            };
            call(
                "verify_with_oracle_proof",
                vec![env, octocat.into_val(env), proof.into_val(env)],
            )
        }
        other => panic!("no dispatcher arm for operation `{other}`; add one to plan()"),
    }
}

#[derive(Debug)]
#[allow(dead_code)] // `Host` payload is read through `Debug` in failure reports.
enum Outcome {
    Ok,
    Contract(ContractError),
    Host(std::string::String),
}

fn invoke(fx: &Fixture, actor: Option<&Address>, call: &Call, mock_all: bool) -> Outcome {
    let env = &fx.env;
    let contract = call
        .contract
        .clone()
        .unwrap_or_else(|| fx.contract_id.clone());
    let invoke = MockAuthInvoke {
        contract: &contract,
        fn_name: &call.func,
        args: call.args.clone(),
        sub_invokes: &[],
    };
    if mock_all {
        env.mock_all_auths();
    } else {
        let signers: std::vec::Vec<&Address> =
            actor.into_iter().chain(call.co_signers.iter()).collect();
        let auths: std::vec::Vec<MockAuth> = signers
            .into_iter()
            .map(|address| MockAuth {
                address,
                invoke: &invoke,
            })
            .collect();
        env.mock_auths(&auths);
    }
    let res = env.try_invoke_contract::<Val, ContractError>(
        &contract,
        &Symbol::new(env, &call.func),
        call.args.clone(),
    );
    match res {
        Ok(_) => Outcome::Ok,
        Err(Ok(e)) => Outcome::Contract(e),
        Err(Err(e)) => Outcome::Host(std::format!("{e:?}")),
    }
}

fn run_row(row: &Row, mock_all: bool) -> Outcome {
    let fx = Fixture::new();
    let actor = fx.actor(&row.actor);
    let caller = actor.clone().unwrap_or_else(|| fx.random.clone());
    let call = plan(&fx, &row.operation, &caller);
    invoke(&fx, actor.as_ref(), &call, mock_all)
}

#[test]
fn auth_matrix_csv_rows_all_pass() {
    let rows = rows();
    // Rows are independent (fresh `Env` each), so spread them across threads.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let failures: std::vec::Vec<std::string::String> = std::thread::scope(|s| {
        rows.chunks(rows.len().div_ceil(threads))
            .map(|chunk| {
                s.spawn(move || {
                    chunk
                        .iter()
                        .filter_map(check_row)
                        .collect::<std::vec::Vec<_>>()
                })
            })
            .collect::<std::vec::Vec<_>>()
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });
    assert!(
        failures.is_empty(),
        "{} of {} auth matrix rows failed:\n{}",
        failures.len(),
        rows.len(),
        failures.join("\n")
    );
}

/// Runs one row; returns a failure line citing the CSV line on mismatch or panic.
fn check_row(row: &Row) -> Option<std::string::String> {
    let result = std::panic::catch_unwind(|| {
        let got = run_row(row, false);
        let ok = match (&row.expected, &got) {
            (None, Outcome::Ok) => true,
            (Some(want), Outcome::Contract(e)) => std::format!("{e:?}") == *want,
            // A fixed-identity `require_auth` fails in the host, not with a
            // `ContractError`; prove it is auth by replaying with all auths.
            (Some(want), Outcome::Host(_)) if want == "NotAuthorized" => {
                !matches!(run_row(row, true), Outcome::Host(_))
            }
            _ => false,
        };
        (!ok).then(|| std::format!("got {got:?}"))
    });
    let detail = match result {
        Ok(None) => return None,
        Ok(Some(detail)) => detail,
        Err(panic) => std::format!(
            "panicked: {}",
            panic
                .downcast_ref::<std::string::String>()
                .map(std::string::String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("<non-string panic>")
        ),
    };
    Some(std::format!(
        "auth_matrix.csv line {}: {} as {} expected {} but {}",
        row.line,
        row.operation,
        row.actor,
        row.expected.as_deref().unwrap_or("Ok"),
        detail
    ))
}

/// Guard: every entrypoint whose body calls `require_auth` needs a CSV row.
#[test]
fn every_auth_gated_entrypoint_has_a_matrix_row() {
    let bodies: std::vec::Vec<&str> = CONTRACT_SRCS
        .iter()
        .map(|src| {
            let start = src
                .find("#[contractimpl]\nimpl TrustBridgeContract {")
                .expect("contractimpl block not found");
            &src[start..]
        })
        .collect();
    let covered: std::collections::BTreeSet<std::string::String> =
        rows().into_iter().map(|r| r.operation).collect();
    let missing: std::vec::Vec<&str> = bodies
        .iter()
        .flat_map(|body| body.split("\n    pub fn ").skip(1))
        .filter(|f| f.contains("require_auth"))
        .map(|f| f.split('(').next().unwrap())
        .filter(|name| !covered.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "auth-gated entrypoints missing from tests/auth_matrix.csv: {missing:?}"
    );
}
