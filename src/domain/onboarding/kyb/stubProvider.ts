import { config } from '../../../config';
import type { OnboardingApplication, RejectionDetail } from '../../../contract';
import {
  KYB_CHECK_KEYS,
  type KybCheckKey,
  type KybCheckResult,
  type KybOutcome,
  type KybProvider,
} from './provider';

const wait = (ms: number): Promise<void> =>
  ms <= 0 ? Promise.resolve() : new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Deterministic stand-in for real KYB. Every check passes unless the
 * application data trips one of these documented triggers — which exist so the
 * rejected path is exercisable in dev and tests:
 *
 *   - cac_lookup       fails if the legal name contains "REJECT"
 *   - director_identity fails if any principal BVN is "00000000000"
 *   - sanctions_pep    fails if any principal full name contains "SANCTION"
 *
 * adverse_media and risk_rating always pass.
 */
function evaluate(application: OnboardingApplication): {
  results: KybCheckResult[];
  rejectionReasons: RejectionDetail[];
} {
  const legalName = application.business?.legalName ?? '';
  const principals = application.principals;

  const cacFails = /reject/i.test(legalName);
  const bvnFails = principals.some((p) => p.bvn === '00000000000');
  const sanctionsFails = principals.some((p) => /sanction/i.test(p.fullName));

  const verdicts: Record<KybCheckKey, { passed: boolean; detail: string }> = {
    cac_lookup: cacFails
      ? { passed: false, detail: 'RC number could not be verified with the Corporate Affairs Commission.' }
      : { passed: true, detail: 'RC number verified with the Corporate Affairs Commission.' },
    director_identity: bvnFails
      ? { passed: false, detail: 'A director’s BVN did not resolve at NIBSS.' }
      : { passed: true, detail: 'Director identities cross-referenced with NIBSS.' },
    sanctions_pep: sanctionsFails
      ? { passed: false, detail: 'A principal matched a sanctions / PEP list entry.' }
      : { passed: true, detail: 'No matches on OFAC SDN, EU Consolidated, or UN sanctions lists.' },
    adverse_media: { passed: true, detail: 'No adverse media or enforcement records found.' },
    risk_rating: { passed: true, detail: 'Segment, corridor, and volume risk model applied.' },
  };

  const results = KYB_CHECK_KEYS.map((key) => ({ key, ...verdicts[key] }));

  const rejectionReasons: RejectionDetail[] = [];
  if (cacFails) rejectionReasons.push({ field: 'business.cacNumber', reason: verdicts.cac_lookup.detail });
  if (bvnFails) rejectionReasons.push({ field: 'principals[].bvn', reason: verdicts.director_identity.detail });
  if (sanctionsFails)
    rejectionReasons.push({ field: 'principals[].fullName', reason: verdicts.sanctions_pep.detail });

  return { results, rejectionReasons };
}

export const stubKybProvider: KybProvider = {
  async runChecks(application): Promise<KybOutcome> {
    const { results, rejectionReasons } = evaluate(application);

    // Walk the checks with a little latency each, like a real pipeline.
    for (let i = 0; i < results.length; i++) {
      await wait(config.kybCheckDelayMs);
    }

    return {
      approved: rejectionReasons.length === 0,
      checks: results,
      rejectionReasons,
    };
  },
};
