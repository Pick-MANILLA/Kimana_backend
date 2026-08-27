import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { dirname, resolve, sep } from 'node:path';
import { config } from '../config';
import type { DocumentStore } from './documentStore';

const root = resolve(config.storageDir);

function pathFor(key: string): string {
  const full = resolve(root, key);
  if (full !== root && !full.startsWith(root + sep)) {
    throw new Error(`storage key escapes root: ${key}`);
  }
  return full;
}

export const filesystemDocumentStore: DocumentStore = {
  async put(key, body) {
    const path = pathFor(key);
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, body);
  },
  async get(key) {
    return readFile(pathFor(key));
  },
  async delete(key) {
    await rm(pathFor(key), { force: true });
  },
};
