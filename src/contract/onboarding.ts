import type { Id, ISODate, ISODateTime, Money } from './common';

export type OnboardingStatus = 'draft' | 'submitted' | 'in_review' | 'approved' | 'rejected';

export interface Address {
  readonly line1?: string;
  readonly line2?: string;
  readonly city?: string;
  readonly state: string;
  readonly postalCode?: string;
  /** ISO 3166-1 alpha-2 */
  readonly country: string;
}

export type BusinessType =
  | 'sole_proprietorship'
  | 'limited_liability_company'
  | 'partnership'
  | 'public_limited_company';

export type IndustrySector =
  | 'agriculture_agro_export'
  | 'textiles_apparel'
  | 'solid_minerals'
  | 'manufacturing'
  | 'oil_gas_services'
  | 'technology'
  | 'trading_commodities'
  | 'other';

export interface BusinessDetails {
  readonly legalName: string;
  readonly tradingName?: string;
  readonly cacNumber: string;
  readonly businessType: BusinessType;
  readonly industry: IndustrySector;
  readonly tradingAddress: Address;
  /** ISO 3166-1 alpha-2 */
  readonly countryOfIncorporation: string;
}

export interface DirectorOrBeneficialOwner {
  readonly id: Id;
  readonly fullName: string;
  readonly role: 'director' | 'beneficial_owner' | 'both';
  /** Required when role is 'beneficial_owner' or 'both'. */
  readonly ownershipPercentage?: number;
  /** Required when role is 'director' or 'both'. */
  readonly dateOfBirth?: ISODate;
  /** Bank Verification Number — 11 digits. Required when role is 'director' or 'both'. */
  readonly bvn?: string;
  /** National Identification Number — 11 digits. Required when role is 'director' or 'both'. */
  readonly nin?: string;
}

export type OnboardingDocumentType =
  | 'cac_certificate'
  | 'memart'
  | 'proof_of_address'
  | 'directors_id'
  | 'board_resolution';

export type DocumentUploadStatus = 'pending' | 'uploading' | 'uploaded' | 'failed';

export interface UploadedDocument {
  readonly id: Id;
  readonly type: OnboardingDocumentType;
  readonly fileName: string;
  readonly mimeType: string;
  readonly sizeBytes: number;
  readonly status: DocumentUploadStatus;
  readonly uploadProgressPercent: number;
  readonly uploadedAt?: ISODateTime;
  readonly errorMessage?: string;
}

export interface RejectionDetail {
  /** Dotted path into the application, e.g. "business.cacNumber", "principals[0].bvn". */
  readonly field: string;
  readonly reason: string;
}

/**
 * Populated only once status === 'approved'. The account-level facts shown
 * on the approval screen — distinct from the application data that produced
 * them (e.g. riskRatingLabel is a human-readable ops judgment, not the same
 * as the internal RiskRating used for screening decisions).
 */
export interface ApprovedAccountSummary {
  readonly accountId: string;
  readonly riskRatingLabel: string;
  readonly segment: string;
  /** e.g. "NGN → USD / EUR" */
  readonly corridor: string;
  readonly monthlyLimit: Money;
}

export interface OnboardingApplication {
  readonly id: Id;
  readonly customerId: Id;
  readonly status: OnboardingStatus;
  readonly business: BusinessDetails | null;
  readonly principals: readonly DirectorOrBeneficialOwner[];
  readonly documents: readonly UploadedDocument[];
  /** Present only when status === 'rejected'. */
  readonly rejectionReasons?: readonly RejectionDetail[];
  /** Present only when status === 'approved'. */
  readonly approvedSummary?: ApprovedAccountSummary;
  readonly submittedAt?: ISODateTime;
  readonly reviewedAt?: ISODateTime;
}

export interface DocumentFileInput {
  readonly type: OnboardingDocumentType;
  readonly fileName: string;
  readonly mimeType: string;
  readonly sizeBytes: number;
  readonly data: Blob;
}

export interface OnboardingApi {
  getApplication(customerId: Id): Promise<OnboardingApplication>;
  saveBusinessDetails(applicationId: Id, business: BusinessDetails): Promise<OnboardingApplication>;
  savePrincipals(
    applicationId: Id,
    principals: readonly DirectorOrBeneficialOwner[],
  ): Promise<OnboardingApplication>;
  uploadDocument(
    applicationId: Id,
    file: DocumentFileInput,
    onProgress?: (percent: number) => void,
  ): Promise<UploadedDocument>;
  retryDocumentUpload(applicationId: Id, documentId: Id): Promise<UploadedDocument>;
  removeDocument(applicationId: Id, documentId: Id): Promise<void>;
  /** Kicks off KYB verification; resolves once checks complete with the final status. */
  submit(applicationId: Id): Promise<OnboardingApplication>;
}
