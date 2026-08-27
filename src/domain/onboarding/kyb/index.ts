import { stubKybProvider } from './stubProvider';
import type { KybProvider } from './provider';

/** The active KYB provider. Swap the right-hand side for a real integration. */
export const kybProvider: KybProvider = stubKybProvider;

export type { KybProvider, KybOutcome, KybCheckResult, KybCheckKey } from './provider';
