import type { OnboardingApplication, RejectionDetail } from '../../../contract';

// The five checks match Kimana_frontend verificationCopy.checks (display order).
export type KybCheckKey =
  | 'cac_lookup'
  | 'director_identity'
  | 'sanctions_pep'
  | 'adverse_media'
  | 'risk_rating';

export const KYB_CHECK_KEYS: readonly KybCheckKey[] = [
  'cac_lookup',
  'director_identity',
  'sanctions_pep',
  'adverse_media',
  'risk_rating',
];

export interface KybCheckResult {
  readonly key: KybCheckKey;
  readonly passed: boolean;
  readonly detail?: string;
}

export interface KybOutcome {
  readonly approved: boolean;
  readonly checks: readonly KybCheckResult[];
  /** Populated when `approved` is false — one entry per failed check. */
  readonly rejectionReasons: readonly RejectionDetail[];
}

/**
 * Runs KYB verification for a submitted application. The stub implementation
 * backs the P1 slice; a real provider (CAC lookup, NIBSS/BVN, sanctions/PEP,
 * adverse media) slots in behind this interface with no change to the service.
 */
export interface KybProvider {
  runChecks(application: OnboardingApplication): Promise<KybOutcome>;
}
