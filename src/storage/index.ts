import { filesystemDocumentStore } from './filesystemDocumentStore';
import type { DocumentStore } from './documentStore';

/** The active DocumentStore. Swap the right-hand side to move off the filesystem. */
export const documentStore: DocumentStore = filesystemDocumentStore;

export type { DocumentStore } from './documentStore';
