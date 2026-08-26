use soroban_sdk::{symbol_short, Address, Env, String, Symbol, Vec};

use crate::ContractError;

pub const REG_KEY: Symbol = symbol_short!("reg");
pub const ADMIN_KEY: Symbol = symbol_short!("admin");
pub const COUNT_KEY: Symbol = symbol_short!("count");
pub const VCOUNT_KEY: Symbol = symbol_short!("vcount");
pub const INDEX_KEY: Symbol = symbol_short!("idx");
pub const ORG_INDEX_KEY: Symbol = symbol_short!("orgidx");
pub const TEAM_INDEX_KEY: Symbol = symbol_short!("tmidx");

/// Distinguishes personal accounts from organization and team entries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[soroban_sdk::contracttype]
pub enum EntityType {
    Personal = 0,
    Org = 1,
    Team = 2,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct ContributorRecord {
    pub stellar_address: Address,
    pub registered_at: u64,
    pub verified: bool,
    pub entity_type: EntityType,
    pub org_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct Stats {
    pub total: u32,
    pub verified: u32,
}

pub fn require_initialized(env: &Env) -> Result<(), ContractError> {
    if env.storage().instance().has(&ADMIN_KEY) {
        Ok(())
    } else {
        Err(ContractError::NotInitialized)
    }
}

pub fn get_admin(env: &Env) -> Result<Address, ContractError> {
    require_initialized(env)?;
    env.storage()
        .instance()
        .get(&ADMIN_KEY)
        .ok_or(ContractError::NotInitialized)
}

pub fn get_record(env: &Env, github_username: &String) -> Option<ContributorRecord> {
    env.storage()
        .persistent()
        .get(&(REG_KEY, github_username.clone()))
}

pub fn set_record(env: &Env, github_username: &String, record: &ContributorRecord) {
    env.storage()
        .persistent()
        .set(&(REG_KEY, github_username.clone()), record);
}

pub fn remove_record(env: &Env, github_username: &String) {
    env.storage()
        .persistent()
        .remove(&(REG_KEY, github_username.clone()));
}

pub fn get_count(env: &Env) -> u32 {
    env.storage().instance().get(&COUNT_KEY).unwrap_or(0)
}

pub fn set_count(env: &Env, count: u32) {
    env.storage().instance().set(&COUNT_KEY, &count);
}

pub fn get_verified_count(env: &Env) -> u32 {
    env.storage().instance().get(&VCOUNT_KEY).unwrap_or(0)
}

pub fn set_verified_count(env: &Env, count: u32) {
    env.storage().instance().set(&VCOUNT_KEY, &count);
}

pub fn get_index(env: &Env) -> Vec<String> {
    env.storage()
        .instance()
        .get(&INDEX_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_index(env: &Env, index: &Vec<String>) {
    env.storage().instance().set(&INDEX_KEY, index);
}

pub fn add_to_index(env: &Env, github_username: &String) {
    let mut index = get_index(env);
    index.push_back(github_username.clone());
    set_index(env, &index);
}

pub fn remove_from_index(env: &Env, github_username: &String) {
    let index = get_index(env);
    let mut next = Vec::new(env);
    for i in 0..index.len() {
        let username = index.get(i).unwrap();
        if username != *github_username {
            next.push_back(username);
        }
    }
    set_index(env, &next);
}

pub fn get_stats(env: &Env) -> Stats {
    Stats {
        total: get_count(env),
        verified: get_verified_count(env),
    }
}

pub fn get_org_index(env: &Env) -> Vec<String> {
    env.storage()
        .instance()
        .get(&ORG_INDEX_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_org_index(env: &Env, index: &Vec<String>) {
    env.storage().instance().set(&ORG_INDEX_KEY, index);
}

pub fn add_to_org_index(env: &Env, org_name: &String) {
    let mut index = get_org_index(env);
    index.push_back(org_name.clone());
    set_org_index(env, &index);
}

pub fn remove_from_org_index(env: &Env, org_name: &String) {
    let index = get_org_index(env);
    let mut next = Vec::new(env);
    for i in 0..index.len() {
        let name = index.get(i).unwrap();
        if name != *org_name {
            next.push_back(name);
        }
    }
    set_org_index(env, &next);
}

pub fn get_team_index(env: &Env) -> Vec<String> {
    env.storage()
        .instance()
        .get(&TEAM_INDEX_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_team_index(env: &Env, index: &Vec<String>) {
    env.storage().instance().set(&TEAM_INDEX_KEY, index);
}

pub fn team_key(env: &Env, org_name: &str, team_name: &str) -> String {
    let prefix = String::from_str(env, org_name);
    let suffix = String::from_str(env, team_name);
    prefix.concat(&suffix.concat(&String::from_str(env, ":")))
}

pub fn add_to_team_index(env: &Env, key: &String) {
    let mut index = get_team_index(env);
    index.push_back(key.clone());
    set_team_index(env, &index);
}

pub fn remove_from_team_index(env: &Env, key: &String) {
    let index = get_team_index(env);
    let mut next = Vec::new(env);
    for i in 0..index.len() {
        let k = index.get(i).unwrap();
        if k != *key {
            next.push_back(k);
        }
    }
    set_team_index(env, &next);
}
