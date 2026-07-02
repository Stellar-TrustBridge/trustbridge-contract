# Testnet Checklist

Before sharing a testnet deployment, confirm the basic registry lifecycle.

## Contract flow

- Build the optimized WASM.
- Deploy to Stellar testnet.
- Initialize with the intended admin address.
- Register a sample GitHub username and Stellar address.
- Verify the sample contributor as admin.
- Remove the sample registration and confirm lookup returns empty.

## Metadata

Store the contract ID, network passphrase, deployer address, admin address, and commit hash used for the deployment.
