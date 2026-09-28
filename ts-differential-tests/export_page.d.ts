import { xdr } from '@stellar/stellar-sdk';

export const EXPORT_PAGE_LAYOUT_VERSION: number;

export interface ContributorRecord {
  stellar_address: string;
  payout_address: string;
  registered_at: number;
  verified: boolean;
  is_bot: boolean;
}

export type ExportRecord = [string, ContributorRecord];

export interface ExportPage {
  records: ExportRecord[];
  next_cursor: string | null;
  total: number;
  merkle_root: string;
  has_more: boolean;
}

export function parseContributorRecord(val: xdr.ScVal | unknown): ContributorRecord;
export function parseExportRecord(entry: unknown): ExportRecord;
export function parseExportPage(val: xdr.ScVal | unknown): ExportPage;
export function decodeExportPageFromXdr(xdrBase64: string): ExportPage;
