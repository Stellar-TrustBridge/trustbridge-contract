use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotAuthorized = 3,
    NotRegistered = 4,
    AlreadyVerified = 5,
}

impl ContractError {
    pub fn code(self) -> u32 {
        self as u32
    }
}
