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

/**
 * Extract a u32 from an ScVal.
 */
function scValToU32(scVal) {
  if (scVal.switch().name !== 'scvU32') {
    throw new Error(`Expected scvU32, got ${scVal.switch().name}`);
  }
  return scVal.value();
}

// ---------------------------------------------------------------------------
// Test suite
// ---------------------------------------------------------------------------

describe('XDR differential fixtures', () => {

  // ── Fixture 1: ContributorRecord / get_address ───────────────────────────

  it('get_address_octocat: ContributorRecord decodes to expected Stellar address', (t) => {
    const xdrStr = loadFixture('get_address_octocat');
    const goldenAddr = loadGoldenAddress('get_address_octocat');

    if (xdrStr === null || goldenAddr === null) {
      t.skip('Fixture is a placeholder — run `make xdr-fixtures` to generate real fixtures');
      return;
    }

    const scVal = decodeScVal(xdrStr);

    // ContributorRecord is serialized as ScValMap; stellar_address is the
    // first field. We look it up by name for robustness against field reordering.
    const fields = scValToFields(scVal);

    assert.ok(
      'stellar_address' in fields,
      'ContributorRecord must have a stellar_address field'
    );

    const decodedAddr = scValToAddress(fields['stellar_address']);
    assert.equal(
      decodedAddr,
      goldenAddr,
      `Decoded address ${decodedAddr} must match golden ${goldenAddr}`
    );

    // registered_at must be a u32 (Issue #67: u64→u32 migration).
    assert.ok(
      'registered_at' in fields,
      'ContributorRecord must have a registered_at field'
    );
    assert.equal(
      fields['registered_at'].switch().name,
      'scvU32',
      'registered_at must be encoded as u32, not u64 (ABI regression guard)'
    );

    // verified must be a bool.
    assert.ok(
      'verified' in fields,
      'ContributorRecord must have a verified field'
    );
    assert.equal(
      fields['verified'].switch().name,
      'scvBool',
      'verified must be encoded as bool'
    );
  });

  // ── Fixture 2: BatchRemoveProposedEvent ───────────────────────────────────

  it('batch_remove_proposed_event: event data decodes with correct proposed_by and count', (t) => {
    const xdrStr = loadFixture('batch_remove_proposed_event');
    const meta = loadMeta('batch_remove_proposed_event');

    if (xdrStr === null) {
      t.skip('Fixture is a placeholder — run `make xdr-fixtures` to generate real fixtures');
      return;
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
  });

  // ── Fixture 3: BatchRemoveExecutedEvent ──────────────────────────────────

  it('batch_remove_executed_event: event data decodes with correct executed_by, proposed_by, count, and successful', (t) => {
    const xdrStr = loadFixture('batch_remove_executed_event');
    const meta = loadMeta('batch_remove_executed_event');

    if (xdrStr === null) {
      t.skip('Fixture is a placeholder — run `make xdr-fixtures` to generate real fixtures');
      return;
    }

    const scVal = decodeScVal(xdrStr);
    const fields = scValToFields(scVal);

    // Required fields.
    for (const field of ['executed_by', 'proposed_by']) {
      assert.ok(field in fields, `BatchRemoveExecutedEvent must have '${field}'`);
      assert.equal(
        fields[field].switch().name,
        'scvAddress',
        `${field} must be encoded as ScAddress`
      );
    }

    assert.ok('count' in fields, 'BatchRemoveExecutedEvent must have a count field');
    assert.ok('successful' in fields, 'BatchRemoveExecutedEvent must have a successful field');

    const count = scValToU32(fields['count']);
    const successful = scValToU32(fields['successful']);
    assert.ok(
      successful <= count,
      `successful (${successful}) must be <= count (${count})`
    );

    // executed_by and proposed_by must differ — dual-control invariant.
    const executedBy = scValToAddress(fields['executed_by']);
    const proposedBy = scValToAddress(fields['proposed_by']);
    assert.notEqual(
      executedBy,
      proposedBy,
      'executed_by and proposed_by must be different addresses (dual-control invariant)'
    );

    // domain must be present.
    assert.ok(
      'domain' in fields,
      'BatchRemoveExecutedEvent must carry an EventDomain (Issue #226)'
    );

    if (meta) {
      assert.equal(count, meta.count, `count must match meta (expected ${meta.count})`);
      assert.equal(
        successful,
        meta.successful,
        `successful must match meta (expected ${meta.successful})`
      );
      assert.equal(executedBy, meta.executed_by, 'executed_by must match meta sidecar');
      assert.equal(proposedBy, meta.proposed_by, 'proposed_by must match meta sidecar');
    }
  });

  // ── Fixture 4: BatchRemoveCancelledEvent ─────────────────────────────────

  it('batch_remove_cancelled_event: event data decodes with correct cancelled_by and proposed_by', (t) => {
    const xdrStr = loadFixture('batch_remove_cancelled_event');
    const meta = loadMeta('batch_remove_cancelled_event');

    if (xdrStr === null) {
      t.skip('Fixture is a placeholder — run `make xdr-fixtures` to generate real fixtures');
      return;
    }

    const scVal = decodeScVal(xdrStr);
    const fields = scValToFields(scVal);

    for (const field of ['cancelled_by', 'proposed_by']) {
      assert.ok(field in fields, `BatchRemoveCancelledEvent must have '${field}'`);
      assert.equal(
        fields[field].switch().name,
        'scvAddress',
        `${field} must be encoded as ScAddress`
      );
    }

    assert.ok(
      'timestamp' in fields,
      'BatchRemoveCancelledEvent must have a timestamp field'
    );

    // domain must be present.
    assert.ok(
      'domain' in fields,
      'BatchRemoveCancelledEvent must carry an EventDomain (Issue #226)'
    );

    if (meta) {
      const cancelledBy = scValToAddress(fields['cancelled_by']);
      const proposedBy = scValToAddress(fields['proposed_by']);
      assert.equal(cancelledBy, meta.cancelled_by, 'cancelled_by must match meta sidecar');
      assert.equal(proposedBy, meta.proposed_by, 'proposed_by must match meta sidecar');
    }
  });

  // ── Structural invariants (always run, no fixture file needed) ────────────

  it('XDR ScVal round-trips cleanly through the SDK', () => {
    // A minimal well-known ScVal: u32(42).  If the SDK import is broken the
    // very first call here will throw, catching a missing dep early.
    const u32val = xdr.ScVal.scvU32(42);
    const b64 = u32val.toXDR('base64');
    const decoded = xdr.ScVal.fromXDR(b64, 'base64');
    assert.equal(decoded.switch().name, 'scvU32');
    assert.equal(decoded.value(), 42);
  });

  it('Address round-trips through ScAddress', () => {
    // Use a well-formed contract address (C...) which the SDK handles natively.
    // G-addresses require StrKey decoding through xdr.PublicKey, tested
    // separately in the ContributorRecord fixture decode path.
    const contractBytes = Buffer.alloc(32, 0xAB);
    const addr = Address.contract(contractBytes);
    const scVal = addr.toScVal();
    assert.equal(scVal.switch().name, 'scvAddress');

    // Round-trip: the contract bytes should be preserved.
    const roundTrippedBytes = scVal.value().value();
    assert.deepEqual(
      Buffer.from(roundTrippedBytes),
      contractBytes,
      'Contract address bytes must survive an ScVal round-trip'
    );
  });
});
