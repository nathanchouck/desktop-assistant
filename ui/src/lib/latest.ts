/**
 * Guards against out-of-order responses. Every request takes the next
 * sequence number; a response is applied only if it is newer than the last
 * one applied. Prevents stale search results from flashing while typing.
 */
export class LatestOnly {
  private issued = 0;
  private applied = 0;

  next(): number {
    this.issued += 1;
    return this.issued;
  }

  /** Returns true if a response for `seq` should be applied (and records it). */
  accept(seq: number): boolean {
    if (seq <= this.applied || seq > this.issued) return false;
    this.applied = seq;
    return true;
  }

  /**
   * For a later, fuller answer to request `seq` (results that missed the
   * first response): applied only while `seq` is the newest request.
   */
  acceptUpdate(seq: number): boolean {
    if (seq !== this.issued || seq < this.applied) return false;
    this.applied = seq;
    return true;
  }

  /** Drops every response still in flight (e.g. after clearing the query). */
  invalidate(): void {
    // Skip a number so late updates for the dropped request fail too.
    this.issued += 1;
    this.applied = this.issued;
  }
}
