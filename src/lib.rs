#![no_std]

mod error;
mod events;
mod storage;

pub use error::ContractError;
pub use events::{RegisteredEvent, RemovedEvent, VerifiedEvent};
pub use storage::{ContributorRecord, Stats};

use soroban_sdk::{contract, contractimpl, Address, Env, String, Vec};

use crate::storage::{
    add_to_index, get_admin, get_count, get_index, get_record, get_stats as read_stats, has_record,
    get_verified_count, remove_from_index, remove_record, require_initialized, set_count,
    set_record, set_verified_count, ADMIN_KEY,
};

#[contract]
pub struct TrustBridgeContract;

#[contractimpl]
impl TrustBridgeContract {
    /// Sets the contract admin. Can only be called once.
    pub fn initialize(env: Env, admin: Address) -> Result<(), ContractError> {
        if env.storage().instance().has(&ADMIN_KEY) {
            return Err(ContractError::AlreadyInitialized);
        }

        env.storage().instance().set(&ADMIN_KEY, &admin);
        set_count(&env, 0);
        set_verified_count(&env, 0);

        Ok(())
    }

    /// Registers or updates a GitHub username → Stellar address mapping.
    /// The caller must authenticate as `stellar_address`.
    pub fn register(
        env: Env,
        github_username: String,
        stellar_address: Address,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        stellar_address.require_auth();

        let timestamp = env.ledger().timestamp();
        let existing = get_record(&env, &github_username);

        let record = ContributorRecord {
            stellar_address: stellar_address.clone(),
            registered_at: timestamp,
            verified: existing
                .as_ref()
                .map(|r| r.stellar_address == stellar_address && r.verified)
                .unwrap_or(false),
        };

        if existing.is_none() {
            set_count(&env, get_count(&env).saturating_add(1));
            add_to_index(&env, &github_username);
        } else if let Some(old) = existing {
            if old.stellar_address != stellar_address && old.verified {
                set_verified_count(&env, get_verified_count(&env).saturating_sub(1));
            }
        }

        set_record(&env, &github_username, &record);

        RegisteredEvent {
            github_username: github_username.clone(),
            stellar_address,
            timestamp,
        }
        .publish(&env);

        Ok(())
    }

    /// Read-only lookup. Returns `None` if the username is not registered.
    pub fn get_address(env: Env, github_username: String) -> Option<ContributorRecord> {
        if has_record(&env, &github_username) {
            get_record(&env, &github_username)
        } else {
            None
        }
    }

    /// Removes a registration. Callable by the registrant or the admin.
    ///
    /// `caller` must sign the transaction and must equal either the contract
    /// admin or the registered Stellar address for `github_username`.
    pub fn remove(env: Env, caller: Address, github_username: String) -> Result<(), ContractError> {
        require_initialized(&env)?;

        let record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;
        let admin = get_admin(&env)?;

        caller.require_auth();
        if caller != admin && caller != record.stellar_address {
            return Err(ContractError::NotAuthorized);
        }

        let timestamp = env.ledger().timestamp();
        let stellar_address = record.stellar_address.clone();

        remove_record(&env, &github_username);
        remove_from_index(&env, &github_username);
        set_count(&env, get_count(&env).saturating_sub(1));

        if record.verified {
            set_verified_count(&env, get_verified_count(&env).saturating_sub(1));
        }

        RemovedEvent {
            github_username: github_username.clone(),
            stellar_address,
            timestamp,
        }
        .publish(&env);

        Ok(())
    }

    /// Returns the full registry. Admin-only.
    pub fn get_all_registered(env: Env) -> Result<Vec<(String, Address)>, ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        let index = get_index(&env);
        let mut result = Vec::new(&env);

        for i in 0..index.len() {
            let username = index.get(i).unwrap();
            if let Some(record) = get_record(&env, &username) {
                result.push_back((username, record.stellar_address));
            }
        }

        Ok(result)
    }

    /// Marks a contributor as verified after an off-chain GitHub identity check. Admin-only.
    pub fn verify(env: Env, github_username: String) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        let mut record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;

        if record.verified {
            return Err(ContractError::AlreadyVerified);
        }

        record.verified = true;
        set_record(&env, &github_username, &record);
        set_verified_count(&env, get_verified_count(&env).saturating_add(1));

        let timestamp = env.ledger().timestamp();
        VerifiedEvent {
            github_username: github_username.clone(),
            stellar_address: record.stellar_address.clone(),
            timestamp,
        }
        .publish(&env);

        Ok(())
    }

    /// Returns aggregate registration statistics.
    pub fn get_stats(env: Env) -> Stats {
        read_stats(&env)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env, String};

    fn setup(env: &Env) -> (Address, Address, Address, Address) {
        let admin = Address::generate(env);
        let user = Address::generate(env);
        let other = Address::generate(env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.as_contract(&contract_id, || {
            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        });
        (admin, user, other, contract_id)
    }

    fn username(env: &Env, name: &str) -> String {
        String::from_str(env, name)
    }

    #[test]
    fn test_register_and_get_address_roundtrip() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();

            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(record.stellar_address, user);
            assert!(!record.verified);
        });
    }

    #[test]
    fn test_non_owner_cannot_remove() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();

            let result =
                TrustBridgeContract::remove(env.clone(), other.clone(), username(&env, "octocat"));
            assert_eq!(result, Err(ContractError::NotAuthorized));
        });
    }

    #[test]
    #[should_panic(expected = "Unauthorized function call for address")]
    fn test_admin_functions_reject_non_admin() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });

        env.set_auths(&[]);

        env.as_contract(&contract_id, || {
            let _ = TrustBridgeContract::get_all_registered(env.clone());
        });
    }

    #[test]
    fn test_reregistration_updates_record() {
        let env = Env::default();
        let (_admin, user, new_user, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();

            TrustBridgeContract::verify(env.clone(), username(&env, "octocat")).unwrap();

            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), new_user.clone())
                .unwrap();

            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(record.stellar_address, new_user);
            assert!(!record.verified);

            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 1);
            assert_eq!(stats.verified, 0);
        });
    }

    #[test]
    fn test_get_stats_increments_correctly() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0);
            assert_eq!(stats.verified, 0);

            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
                .unwrap();

            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 2);
            assert_eq!(stats.verified, 0);

            TrustBridgeContract::verify(env.clone(), username(&env, "alice")).unwrap();

            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 2);
            assert_eq!(stats.verified, 1);
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user2.clone(), username(&env, "bob")).unwrap();

            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 1);
            assert_eq!(stats.verified, 1);
        });
    }

    #[test]
    fn test_initialize_only_once() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());

        env.as_contract(&contract_id, || {
            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
            let result = TrustBridgeContract::initialize(env.clone(), admin);
            assert_eq!(result, Err(ContractError::AlreadyInitialized));
        });
    }

    #[test]
    fn test_register_requires_initialization() {
        let env = Env::default();
        let user = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
            );
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }


    #[test]
    fn test_admin_can_remove_registration() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
            TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();

            assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .is_none());
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
        });
    }


    #[test]
    fn test_get_all_registered_returns_indexed_records() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
                .unwrap();

            let all = TrustBridgeContract::get_all_registered(env.clone()).unwrap();
            assert_eq!(all.len(), 2);
            assert_eq!(all.get(0).unwrap(), (username(&env, "alice"), user1));
            assert_eq!(all.get(1).unwrap(), (username(&env, "bob"), user2));
        });
    }


    #[test]
    fn test_removing_verified_record_updates_stats() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), username(&env, "octocat")).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat"))
                .unwrap();

            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0);
            assert_eq!(stats.verified, 0);
        });
    }


    #[test]
    fn test_reregister_same_address_keeps_verification() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), username(&env, "octocat")).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();

            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(record.verified);
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 1);
        });
    }


    #[test]
    fn test_get_address_missing_returns_none() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);

        env.as_contract(&contract_id, || {
            assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "missing")).is_none());
        });
    }


    #[test]
    fn test_verify_missing_registration_fails() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::verify(env.clone(), username(&env, "missing"));
            assert_eq!(result, Err(ContractError::NotRegistered));
        });
    }


    #[test]
    fn test_remove_missing_registration_fails() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "missing"));
            assert_eq!(result, Err(ContractError::NotRegistered));
        });
    }


    #[test]
    fn test_double_verify_fails() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), username(&env, "octocat")).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::verify(env.clone(), username(&env, "octocat"));
            assert_eq!(result, Err(ContractError::AlreadyVerified));
        });
    }


    #[test]
    fn test_register_two_users_keeps_addresses() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone()).unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone()).unwrap();
            assert_eq!(TrustBridgeContract::get_address(env.clone(), username(&env, "alice")).unwrap().stellar_address, user1);
            assert_eq!(TrustBridgeContract::get_address(env.clone(), username(&env, "bob")).unwrap().stellar_address, user2);
        });
    }


    #[test]
    fn test_owner_can_remove_registration() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat")).unwrap();
            assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_none());
        });
    }


    #[test]
    fn test_readding_removed_user_increments_count() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat")).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
        });
    }


    #[test]
    fn test_export_skips_removed_records() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone()).unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone()).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice")).unwrap();
            let all = TrustBridgeContract::get_all_registered(env.clone()).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all.get(0).unwrap(), (username(&env, "bob"), user2));
        });
    }


    #[test]
    fn test_stats_empty_after_setup() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);

        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0);
            assert_eq!(stats.verified, 0);
        });
    }


    #[test]
    fn test_removed_verified_user_can_register_unverified() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), username(&env, "octocat")).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat")).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
            let record = TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(!record.verified);
        });
    }


    #[test]
    fn test_error_codes_match_repr() {
        assert_eq!(ContractError::AlreadyInitialized.code(), 1);
        assert_eq!(ContractError::NotInitialized.code(), 2);
        assert_eq!(ContractError::NotAuthorized.code(), 3);
        assert_eq!(ContractError::NotRegistered.code(), 4);
        assert_eq!(ContractError::AlreadyVerified.code(), 5);
    }


    #[test]
    fn test_updated_registration_preserves_count() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone()).unwrap();
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
        });
    }


    #[test]
    fn test_unverified_update_stays_unverified() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone()).unwrap();
            let record = TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(!record.verified);
        });
    }


    #[test]
    fn test_verified_same_address_reregister_keeps_count() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), username(&env, "octocat")).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 1);
            assert_eq!(stats.verified, 1);
        });
    }


    #[test]
    fn test_verified_address_change_decrements_verified_count() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), username(&env, "octocat")).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone()).unwrap();
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 0);
        });
    }

}
