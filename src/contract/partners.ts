import type { Id, ISODateTime } from './common';

export type PartnerIntegrationKind = 'collection' | 'payout' | 'fx';
export type PartnerHealthStatus = 'healthy' | 'degraded' | 'down';

export interface PartnerStatus {
  readonly id: Id;
  readonly kind: PartnerIntegrationKind;
  readonly name: string;
  readonly status: PartnerHealthStatus;
  readonly lastCheckedAt: ISODateTime;
}

/** Ops-only: internal partner health, never surfaced to the customer app. */
export interface PartnerApi {
  getStatuses(): Promise<readonly PartnerStatus[]>;
}

/** Customer-facing: partner trouble degrades into honest delay messaging, nothing more. */
export interface TransferDelayNotice {
  readonly transferId: Id;
  readonly message: string;
  readonly estimatedResolutionBy?: ISODateTime;
}

export interface CustomerDelayApi {
  getDelayNotice(transferId: Id): Promise<TransferDelayNotice | null>;
}
