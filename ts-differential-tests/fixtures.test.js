/** Decode the Rust-generated get_address simulation golden with the JS SDK. */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { Address, xdr } from '@stellar/stellar-sdk';

const fixturePath = fileURLToPath(
  new URL('../tests/testdata/bindings/get_address_simulate.v1.json', import.meta.url)
);
const fixture = JSON.parse(readFileSync(fixturePath, 'utf8'));

function decodeRecord(encoded) {
  const value = xdr.ScVal.fromXDR(encoded, 'base64');
  if (value.switch().name === 'scvVoid') return null;

  assert.equal(value.switch().name, 'scvMap');
  const fields = Object.fromEntries(value.value().map((entry) => {
    assert.equal(entry.key().switch().name, 'scvSymbol');
    const key = entry.key().value().toString();
    const field = entry.val();
    const type = field.switch().name;
    if (type === 'scvAddress') return [key, Address.fromScAddress(field.value()).toString()];
    if (type === 'scvU32' || type === 'scvBool') return [key, field.value()];
    throw new Error(`Unexpected XDR type for ${key}: ${type}`);
  }));
  return fields;
}

assert.equal(fixture.function, 'get_address');
assert.equal(fixture.version, 1);
assert.deepEqual(
  fixture.cases.map(({ name }) => name).sort(),
  ['not_registered', 'registered']
);

for (const { name, retval_xdr, decoded } of fixture.cases) {
  test(`Rust get_address XDR: ${name}`, () => {
    assert.deepEqual(decodeRecord(retval_xdr), decoded);
  });
}

/**
 * Issue #599: Rust-emitted upgrade, RBAC and multisig event data XDR.
 * The metadata is a fixture envelope; ledger_hint is NOT an event data field.
 */
const eventFixtureNames = [
  'upgraded_event',
  'role_granted_event',
  'role_revoked_event',
  'upgrade_proposed_event',
  'upgrade_approved_event',
  'upgrade_proposal_executed_event',
  'upgrade_proposal_cancelled_event',
];

function decodeEventData(base64) {
  const value = xdr.ScVal.fromXDR(base64.trim(), 'base64');
  assert.equal(value.switch().name, 'scvMap');
  return Object.fromEntries(value.value().map((entry) => {
    assert.equal(entry.key().switch().name, 'scvSymbol');
    const name = entry.key().value().toString();
    const field = entry.val();
    return [name, field.switch().name];
  }));
}

for (const name of eventFixtureNames) {
  test(`Rust event XDR: ${name}`, () => {
    const root = new URL('./fixtures/', import.meta.url);
    const encoded = readFileSync(new URL(`${name}.xdr`, root), 'utf8');
    const meta = JSON.parse(readFileSync(new URL(`${name}.meta.json`, root), 'utf8'));
    assert.equal(meta.fixture_version, 1);
    assert.equal(meta.event_kind, name.replace(/_event$/, ''));
    assert.ok(Number.isInteger(meta.ledger_hint) && meta.ledger_hint > 0);
    const fields = decodeEventData(encoded);
    assert.equal(fields.timestamp, 'scvU64');
    if (name === 'upgraded_event') {
      assert.equal(fields.new_wasm_hash, undefined); // #[topic] field
      assert.equal(fields.domain, 'scvMap');
    } else if (name.startsWith('role_')) {
      assert.equal(fields.address, undefined); // #[topic] field
      assert.equal(fields.admin, 'scvAddress');
      if (name === 'role_granted_event') assert.equal(fields.role, 'scvU32');
    } else {
      assert.equal(fields.proposal_id, undefined); // #[topic] field
      if (name === 'upgrade_approved_event' || name === 'upgrade_proposal_executed_event') {
        assert.equal(fields.approval_count, 'scvU32');
      }
    }
  });
}
