import { z } from 'zod';

// Field rules mirror the frontend forms (BusinessDetailsPage, DirectorsUboPage).
// Conditional-required-by-role on principals is intentionally NOT enforced here
// for the slice — the mock enforces none, the frontend enforces director fields
// client-side, and tightening it server-side is a P1-later task once the
// rejected-application path exists.

const businessTypeValues = [
  'sole_proprietorship',
  'limited_liability_company',
  'partnership',
  'public_limited_company',
] as const;

const industryValues = [
  'agriculture_agro_export',
  'textiles_apparel',
  'solid_minerals',
  'manufacturing',
  'oil_gas_services',
  'technology',
  'trading_commodities',
  'other',
] as const;

const addressSchema = z.object({
  line1: z.string().trim().optional(),
  line2: z.string().trim().optional(),
  city: z.string().trim().optional(),
  state: z.string().trim().min(1, 'Select your primary state of operation.'),
  postalCode: z.string().trim().optional(),
  country: z.string().trim().length(2, 'Use a 2-letter ISO country code.'),
});

export const businessDetailsSchema = z.object({
  legalName: z.string().trim().min(2, 'Enter your registered business name.'),
  tradingName: z.string().trim().optional(),
  cacNumber: z
    .string()
    .trim()
    .regex(/^RC-?\d{4,8}$/i, 'Enter a valid RC number, e.g. RC-1234567.'),
  businessType: z.enum(businessTypeValues),
  industry: z.enum(industryValues),
  tradingAddress: addressSchema,
  countryOfIncorporation: z.string().trim().length(2, 'Use a 2-letter ISO country code.'),
});

export const principalSchema = z.object({
  id: z.string().trim().min(1).optional(),
  fullName: z.string().trim().min(2, 'Enter the full name.'),
  role: z.enum(['director', 'beneficial_owner', 'both']),
  ownershipPercentage: z.number().min(0).max(100).optional(),
  dateOfBirth: z
    .string()
    .regex(/^\d{4}-\d{2}-\d{2}$/, 'Date of birth must be an ISO date (YYYY-MM-DD).')
    .optional(),
  bvn: z.string().regex(/^\d{11}$/, 'BVN must be 11 digits.').optional(),
  nin: z.string().regex(/^\d{11}$/, 'NIN must be 11 digits.').optional(),
});

export const saveBusinessBodySchema = z.object({
  applicationId: z.string().trim().min(1).optional(),
  business: businessDetailsSchema,
});

export const savePrincipalsBodySchema = z.object({
  applicationId: z.string().trim().min(1).optional(),
  principals: z.array(principalSchema),
});

export const submitBodySchema = z
  .object({ applicationId: z.string().trim().min(1).optional() })
  .optional();

export type BusinessDetailsInput = z.infer<typeof businessDetailsSchema>;
export type PrincipalInput = z.infer<typeof principalSchema>;
