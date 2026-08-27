/**
 * Byte storage for uploaded documents. The filesystem implementation backs the
 * P1 slice; a MinIO / S3 implementation slots in behind this same interface
 * (config gains a bucket, docker-compose gains a service) with no change to
 * callers.
 */
export interface DocumentStore {
  put(key: string, body: Buffer, contentType: string): Promise<void>;
  get(key: string): Promise<Buffer>;
  delete(key: string): Promise<void>;
}
