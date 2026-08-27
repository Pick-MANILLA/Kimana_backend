import type { Id, ISODateTime } from './common';

export type TradeDocumentType = 'form_m' | 'form_q' | 'paar' | 'bill_of_lading' | 'commercial_invoice';

export type TradeDocumentReviewStatus = 'pending_upload' | 'submitted' | 'approved' | 'replacement_requested';

export interface TradeDocument {
  readonly id: Id;
  readonly transferId: Id;
  readonly type: TradeDocumentType;
  readonly status: TradeDocumentReviewStatus;
  readonly fileName?: string;
  readonly uploadedAt?: ISODateTime;
  /** Set when status === 'replacement_requested'. */
  readonly reviewNote?: string;
}

export interface DocumentChecklistItem {
  readonly type: TradeDocumentType;
  readonly required: boolean;
  readonly satisfied: boolean;
}

export interface TradeDocumentFileInput {
  readonly fileName: string;
  readonly mimeType: string;
  readonly sizeBytes: number;
  readonly data: Blob;
}

export interface TradeDocumentApi {
  getChecklist(transferId: Id): Promise<readonly DocumentChecklistItem[]>;
  listDocuments(transferId: Id): Promise<readonly TradeDocument[]>;
  uploadDocument(
    transferId: Id,
    type: TradeDocumentType,
    file: TradeDocumentFileInput,
    onProgress?: (percent: number) => void,
  ): Promise<TradeDocument>;
}

export interface OpsTradeDocumentApi {
  review(
    documentId: Id,
    decision: { outcome: 'approve' | 'request_replacement'; note?: string },
  ): Promise<TradeDocument>;
}
