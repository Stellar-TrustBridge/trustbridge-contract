#![no_std]

mod error;
mod events;
mod storage;

pub use error::ContractError;
pub use events::{RegisteredEvent, RemovedEvent, VerifiedEvent};
pub use storage::{ContributorRecord, EntityType, Stats};

use soroban_sdk::{contract, contractimpl, Address, Env, String, Vec};

use crate::storage::{
    add_to_index, add_to_org_index, add_to_team_index, get_admin, get_count, get_index,
    get_record, get_stats as read_stats, get_verified_count, remove_from_index,
    remove_from_org_index, remove_from_team_index, remove_record, require_initialized, set_count,
    set_record, set_verified_count, team_key, ADMIN_KEY,
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
    /// `entity_type`: 0 = Personal, 1 = Org, 2 = Team.
    /// For teams, `org_name` must be provided.
    pub fn register(
        env: Env,
        github_username: String,
        stellar_address: Address,
        entity_type: u32,
        org_name: Option<String>,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        stellar_address.require_auth();

        let etype = match entity_type {
            0 => EntityType::Personal,
            1 => EntityType::Org,
            2 => EntityType::Team,
            _ => return Err(ContractError::InvalidEntityType),
        };

        if etype == EntityType::Team && org_name.is_none() {
            return Err(ContractError::OrgNameRequired);
        }

        let timestamp = env.ledger().timestamp();
        let existing = get_record(&env, &github_username);

        let record = ContributorRecord {
            stellar_address: stellar_address.clone(),
            registered_at: timestamp,
            verified: existing
                .as_ref()
                .map(|r| r.stellar_address == stellar_address && r.verified)
                .unwrap_or(false),
            entity_type: etype,
            org_name: org_name.clone(),
        };

        if existing.is_none() {
            set_count(&env, get_count(&env).saturating_add(1));
            add_to_index(&env, &github_username);
            match etype {
                EntityType::Org => {
                    if let Some(ref oname) = org_name {
                        add_to_org_index(&env, oname);
                    }
                }
                EntityType::Team => {
                    if let Some(ref oname) = org_name {
                        let tk = team_key(&env, oname, &github_username);
                        add_to_team_index(&env, &tk);
                    }
                }
                _ => {}
            }
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
        get_record(&env, &github_username)
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

        match record.entity_type {
            EntityType::Org => {
                if let Some(ref oname) = record.org_name {
                    remove_from_org_index(&env, oname);
                }
            }
            EntityType::Team => {
                if let Some(ref oname) = record.org_name {
                    let tk = team_key(&env, oname, &github_username);
                    remove_from_team_index(&env, &tk);
                }
            }
            _ => {}
        }

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

    fn register_personal(env: &Env, contract_id: &soroban_sdk::Address, name: &str, addr: &Address) {
        TrustBridgeContract::register(
            env.clone(),
            username(env, name),
            addr.clone(),
            0,
            None,
        )
        .unwrap();
    }

    fn register_org(
        env: &Env,
        contract_id: &soroban_sdk::Address,
        name: &str,
        addr: &Address,
        org: &str,
    ) {
        TrustBridgeContract::register(
            env.clone(),
            username(env, name),
            addr.clone(),
            1,
            Some(username(env, org)),
        )
        .unwrap();
    }

    fn register_team(
        env: &Env,
        contract_id: &soroban_sdk::Address,
        name: &str,
        addr: &Address,
        org: &str,
    ) {
        TrustBridgeContract::register(
            env.clone(),
            username(env, name),
            addr.clone(),
            2,
            Some(username(env, org)),
        )
        .unwrap();
    }

    #[test]
    fn test_register_and_get_address_roundtrip() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            register_personal(&env, &contract_id, "octocat", &user);

            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(record.stellar_address, user);
            assert!(!record.verified);
            assert_eq!(record.entity_type, EntityType::Personal);
        });
    }

    #[test]
    fn test_non_owner_cannot_remove() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            register_personal(&env, &contract_id, "octocat", &user);

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
            register_personal(&env, &contract_id, "octocat", &user);
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
            register_personal(&env, &contract_id, "octocat", &user);
            TrustBridgeContract::verify(env.clone(), username(&env, "octocat")).unwrap();
            register_personal(&env, &contract_id, "octocat", &new_user);

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

            register_personal(&env, &contract_id, "alice", &user1);
            register_personal(&env, &contract_id, "bob", &user2);

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
    fn test_register_org_entry() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            register_org(&env, &contract_id, "stellar-org", &user, "stellar-org");

            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "stellar-org"))
                    .unwrap();
            assert_eq!(record.entity_type, EntityType::Org);
            assert_eq!(
                record.org_name,
                Some(username(&env, "stellar-org"))
            );
        });
    }

    #[test]
    fn test_register_team_entry() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            register_team(
                &env,
                &contract_id,
                "engineering-team",
                &user,
                "stellar-org",
            );

            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "engineering-team"))
                    .unwrap();
            assert_eq!(record.entity_type, EntityType::Team);
            assert_eq!(record.org_name, Some(username(&env, "stellar-org")));
        });
    }

    #[test]
    fn test_team_requires_org_name() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::register(
                env.clone(),
                username(&env, "my-team"),
                user.clone(),
                2,
                None,
            );
            assert_eq!(result, Err(ContractError::OrgNameRequired));
        });
    }

    #[test]
    fn test_invalid_entity_type() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::register(
                env.clone(),
                username(&env, "someone"),
                user.clone(),
                99,
                None,
            );
            assert_eq!(result, Err(ContractError::InvalidEntityType));
        });
    }

    #[test]
    fn test_remove_org_cleans_up_index() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            register_org(&env, &contract_id, "my-org", &user, "my-org");
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 1);

            TrustBridgeContract::remove(
                env.clone(),
                user.clone(),
                username(&env, "my-org"),
            )
            .unwrap();

            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0);
        });
    }
}
