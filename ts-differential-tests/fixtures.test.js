/**
 * Differential tests: decode XDR fixtures using the Stellar TypeScript SDK
 * and assert the decoded field values match Rust golden values.
 *
 * Run:  npm test  (or  node --test  from this directory)
 *
 * Fixture files live in ./fixtures/.  Placeholder files (starting with '#')
 * are skipped rather than failed so CI does not break before the Rust
 * generator has been run.  Generate real fixtures with:
 *
 *   make xdr-fixtures        (from the repo root)
 *   # or manually:
 *   cargo test generate_xdr_fixtures -- --ignored --nocapture
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { xdr, Address, StrKey } from '@stellar/stellar-sdk';
import { readFileSync, existsSync } from 'fs';
import { join } from 'path';
import { fileURLToPath } from 'url';

const __dirname = fileURLToPath(new URL('.', import.meta.url));

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Load a fixture file.  Returns null when the file is a placeholder (starts
 * with '#') so individual tests can skip gracefully.
 */
function loadFixture(name) {
  const path = join(__dirname, 'fixtures', `${name}.xdr`);
  if (!existsSync(path)) return null;
  const content = readFileSync(path, 'utf-8').trim();
  return content.startsWith('#') ? null : content;
}

/**
 * Load the golden address sidecar.  Returns null for placeholders.
 */
function loadGoldenAddress(name) {
  const path = join(__dirname, 'fixtures', `${name}.address`);
  if (!existsSync(path)) return null;
  const content = readFileSync(path, 'utf-8').trim();
  return content.startsWith('#') ? null : content;
}

/**
 * Load the meta JSON sidecar that records expected field values.
 * Returns null when the file does not exist or is a placeholder.
 */
function loadMeta(name) {
  const path = join(__dirname, 'fixtures', `${name}.meta.json`);
  if (!existsSync(path)) return null;
  const content = readFileSync(path, 'utf-8').trim();
  if (content.startsWith('#')) return null;
  try {
    return JSON.parse(content);
  } catch {
    return null;
  }
}

/**
 * Decode a base64 XDR ScVal and return the raw xdr.ScVal object.
 */
function decodeScVal(b64) {
  return xdr.ScVal.fromXDR(b64, 'base64');
}

/**
 * Walk an ScVal struct and return a plain object mapping field names to
 * raw ScVal objects.  Works for #[contracttype] structs emitted by
 * soroban-sdk which serialize as ScValMap entries with ScSymbol keys.
 */
function scValToFields(scVal) {
  // Struct types come through as ScValMap.
  if (scVal.switch().name !== 'scvMap') {
    throw new Error(`Expected scvMap, got ${scVal.switch().name}`);
  }
  const result = {};
  for (const entry of scVal.value()) {
    const key = entry.key().value().toString();
    result[key] = entry.val();
  }
  return result;
}

/**
 * Extract the Stellar address string from an ScVal that holds an ScAddress.
 * Works for both account addresses (G...) and contract addresses (C...).
 */
function scValToAddress(scVal) {
  if (scVal.switch().name !== 'scvAddress') {
    throw new Error(`Expected scvAddress, got ${scVal.switch().name}`);
  }
  const scAddr = scVal.value();
  if (scAddr.switch().name === 'scAddressTypeAccount') {
    // AccountID → PublicKey → ed25519 bytes → StrKey G-address
    return StrKey.encodeEd25519PublicKey(scAddr.value().ed25519());
  }
  // Contract address (C...)
  return StrKey.encodeContract(scAddr.value());
}

import {
  parseContributorRecord,
  parseExportRecord,
  parseExportPage,
  decodeExportPageFromXdr,
  EXPORT_PAGE_LAYOUT_VERSION,
} from './export_page.js';

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

    const scVal = decodeScVal(xdrStr);
    const fields = scValToFields(scVal);

    // proposed_by must be an Address.
    assert.ok(
      'proposed_by' in fields,
      'BatchRemoveProposedEvent must have a proposed_by field'
    );
    assert.equal(
      fields['proposed_by'].switch().name,
      'scvAddress',
      'proposed_by must be encoded as ScAddress'
    );

    // count must be a u32.
    assert.ok('count' in fields, 'BatchRemoveProposedEvent must have a count field');
    const count = scValToU32(fields['count']);
    assert.ok(count > 0, 'count must be greater than 0');

    // timestamp must be a u64.
    assert.ok(
      'timestamp' in fields,
      'BatchRemoveProposedEvent must have a timestamp field'
    );
    assert.equal(
      fields['timestamp'].switch().name,
      'scvU64',
      'timestamp must be encoded as u64'
    );

    // domain must be present (EventDomain struct — Issue #226).
    assert.ok(
      'domain' in fields,
      'BatchRemoveProposedEvent must carry an EventDomain (Issue #226)'
    );

    // If a meta sidecar was generated, assert the exact field values.
    if (meta) {
      assert.equal(
        count,
        meta.count,
        `count must match meta sidecar (expected ${meta.count}, got ${count})`
      );
      const proposedBy = scValToAddress(fields['proposed_by']);
      assert.equal(
        proposedBy,
        meta.proposed_by,
        `proposed_by must match meta sidecar`
      );
    }
    
    console.log('✓ TypeScript decode matches Rust golden value');

    // Test ExportPage & ContributorRecord typed parsing
    if (EXPORT_PAGE_LAYOUT_VERSION !== 2) {
      throw new Error(`Expected EXPORT_PAGE_LAYOUT_VERSION to be 2, got ${EXPORT_PAGE_LAYOUT_VERSION}`);
    }

    const testRecordRaw = {
      stellar_address: goldenAddress,
      payout_address: goldenAddress,
      registered_at: 1700000000,
      verified: true,
      is_bot: false,
    };

    const parsedRecord = parseContributorRecord(testRecordRaw);
    if (
      parsedRecord.stellar_address !== goldenAddress ||
      parsedRecord.payout_address !== goldenAddress ||
      parsedRecord.registered_at !== 1700000000 ||
      parsedRecord.verified !== true ||
      parsedRecord.is_bot !== false
    ) {
      throw new Error('parseContributorRecord failed to extract typed fields correctly');
    }

    const testPageRaw = {
      records: [['octocat', testRecordRaw]],
      next_cursor: Buffer.from([1, 2, 3, 4, 5, 6, 7, 8]),
      total: 1,
      merkle_root: Buffer.alloc(32, 0xaa),
      has_more: false,
    };

    const parsedPage = parseExportPage(testPageRaw);
    if (parsedPage.total !== 1 || parsedPage.has_more !== false || !parsedPage.next_cursor || !parsedPage.merkle_root) {
      throw new Error('parseExportPage failed to parse page header');
    }
    if (parsedPage.records.length !== 1 || parsedPage.records[0][0] !== 'octocat') {
      throw new Error('parseExportPage failed to parse records');
    }

    console.log('✓ TypeScript typed export bindings match expected layout v2');
  }
};
