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
