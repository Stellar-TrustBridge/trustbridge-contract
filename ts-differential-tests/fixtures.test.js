/**
 * Differential tests: decode XDR fixtures using TypeScript SDK
 * and compare against Rust contract golden values.
 */

import { xdr, Address } from '@stellar/stellar-sdk';
import { readFileSync } from 'fs';
import { join } from 'path';
import { fileURLToPath } from 'url';

const __dirname = fileURLToPath(new URL('.', import.meta.url));

// Load XDR fixture
function loadFixture(name) {
  const path = join(__dirname, 'fixtures', `${name}.xdr`);
  const content = readFileSync(path, 'utf-8').trim();
  
  // Skip placeholder comments
  if (content.startsWith('#')) {
    throw new Error(
      `Fixture ${name} is a placeholder. Run 'make xdr-fixtures' to generate real fixtures.`
    );
  }
  
  return content;
}

// Load golden address value
function loadGoldenAddress(name) {
  const path = join(__dirname, 'fixtures', `${name}.address`);
  const content = readFileSync(path, 'utf-8').trim();
  
  // Skip placeholder comments
  if (content.startsWith('#')) {
    throw new Error(
      `Golden address ${name} is a placeholder. Run 'make xdr-fixtures' to generate real fixtures.`
    );
  }
  
  return content;
}

// Parse XDR and extract address
function parseAddressFromXdr(xdrString) {
  const scVal = xdr.ScVal.fromXDR(xdrString, 'base64');
  
  // Handle Option<ContributorRecord> - Some(record) or None
  if (scVal.switch().name === 'ScValSome') {
    const record = scVal.value();
    
    // ContributorRecord is a struct with stellar_address as first field
    const stellarAddressBytes = record.value()[0].value().value();
    const address = Address.fromScAddress(stellarAddressBytes);
    return address.toString();
  }
  
  return null;
}

// Test: get_address fixture should decode to expected address
export default {
  async test() {
    console.log('Running differential tests for TypeScript bindings...');
    
    // Test get_address fixture
    const get_address_xdr = loadFixture('get_address_octocat');
    const decodedAddress = parseAddressFromXdr(get_address_xdr);
    
    if (!decodedAddress) {
      throw new Error('Failed to decode address from XDR fixture');
    }
    
    console.log(`Decoded address: ${decodedAddress}`);
    
    // This address should match the golden value in the fixture
    const goldenAddress = loadGoldenAddress('get_address_octocat');
    if (decodedAddress !== goldenAddress) {
      throw new Error(
        `Address mismatch: TS decoded ${decodedAddress} but golden is ${goldenAddress}`
      );
    }
    
    console.log('✓ TypeScript decode matches Rust golden value');
  }
};
