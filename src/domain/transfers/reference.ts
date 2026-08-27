import { randomInt } from 'node:crypto';

// Crockford-ish alphabet, no ambiguous chars. Format: KM-XXXXXX (see contract).
const ALPHABET = 'ABCDEFGHJKMNPQRSTVWXYZ0123456789';

export function generateReference(): string {
  let body = '';
  for (let i = 0; i < 6; i++) body += ALPHABET[randomInt(ALPHABET.length)];
  return `KM-${body}`;
}
