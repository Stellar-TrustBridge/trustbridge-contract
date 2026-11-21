/**
 * Typed bindings and parser helpers for TrustBridge ExportPage and ContributorRecord.
 * Aligned with layout golden v2 (Issue #96, Issue #215, Issue #216).
 */

import { xdr, Address, scValToNative } from '@stellar/stellar-sdk';

/** Documented layout version for ExportPage */
export const EXPORT_PAGE_LAYOUT_VERSION = 2;

/**
 * Parses a ContributorRecord from ScVal or native object.
 *
 * @param {xdr.ScVal | object} val - ScVal or parsed map/object
 * @returns {{
 *   stellar_address: string,
 *   payout_address: string,
 *   registered_at: number,
 *   verified: boolean,
 *   is_bot: boolean
 * }}
 */
export function parseContributorRecord(val) {
  if (!val) {
    throw new Error('Cannot parse null or undefined ContributorRecord');
  }

  // If already parsed native object
  if (typeof val === 'object' && 'stellar_address' in val) {
    const rawStellar = val.stellar_address;
    const rawPayout = val.payout_address;
    return {
      stellar_address: typeof rawStellar === 'string' ? rawStellar : rawStellar?.toString?.() ?? '',
      payout_address: typeof rawPayout === 'string' ? rawPayout : rawPayout?.toString?.() ?? '',
      registered_at: Number(val.registered_at ?? 0),
      verified: Boolean(val.verified),
      is_bot: Boolean(val.is_bot),
    };
  }

  // If ScVal instance
  if (val instanceof xdr.ScVal) {
    const native = scValToNative(val);
    return parseContributorRecord(native);
  }

  // If raw object with map fields
  const native = typeof val.switch === 'function' ? scValToNative(val) : val;
  return {
    stellar_address: native.stellar_address?.toString() ?? '',
    payout_address: native.payout_address?.toString() ?? '',
    registered_at: Number(native.registered_at ?? 0),
    verified: Boolean(native.verified),
    is_bot: Boolean(native.is_bot),
  };
}

/**
 * Parses an ExportRecord tuple: [username, ContributorRecord].
 *
 * @param {Array | object} entry
 * @returns {[string, ReturnType<typeof parseContributorRecord>]}
 */
export function parseExportRecord(entry) {
  if (Array.isArray(entry) && entry.length >= 2) {
    const username = typeof entry[0] === 'string' ? entry[0] : String(entry[0]);
    const record = parseContributorRecord(entry[1]);
    return [username, record];
  }
  throw new Error('ExportRecord must be a 2-element tuple [username, record]');
}

/**
 * Parses an ExportPage from ScVal or decoded native object.
 *
 * @param {xdr.ScVal | object} val
 * @returns {{
 *   records: Array<[string, ReturnType<typeof parseContributorRecord>]>,
 *   next_cursor: string | null,
 *   total: number,
 *   merkle_root: string,
 *   has_more: boolean
 * }}
 */
export function parseExportPage(val) {
  if (!val) {
    throw new Error('Cannot parse null or undefined ExportPage');
  }

  const native = val instanceof xdr.ScVal ? scValToNative(val) : val;

  const rawRecords = Array.isArray(native.records) ? native.records : [];
  const records = rawRecords.map(parseExportRecord);

  let next_cursor = null;
  if (native.next_cursor) {
    if (Buffer.isBuffer(native.next_cursor) || native.next_cursor instanceof Uint8Array) {
      next_cursor = Buffer.from(native.next_cursor).toString('hex');
    } else if (typeof native.next_cursor === 'string') {
      next_cursor = native.next_cursor;
    }
  }

  let merkle_root = '';
  if (native.merkle_root) {
    if (Buffer.isBuffer(native.merkle_root) || native.merkle_root instanceof Uint8Array) {
      merkle_root = Buffer.from(native.merkle_root).toString('hex');
    } else if (typeof native.merkle_root === 'string') {
      merkle_root = native.merkle_root;
    }
  }

  return {
    records,
    next_cursor,
    total: Number(native.total ?? 0),
    merkle_root,
    has_more: Boolean(native.has_more),
  };
}

/**
 * Decodes base64 XDR of an ScVal ExportPage into typed JS representation.
 *
 * @param {string} xdrBase64
 * @returns {ReturnType<typeof parseExportPage>}
 */
export function decodeExportPageFromXdr(xdrBase64) {
  const scVal = xdr.ScVal.fromXDR(xdrBase64, 'base64');
  return parseExportPage(scVal);
}
