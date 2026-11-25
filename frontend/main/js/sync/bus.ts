/**
 * Tells pages when show data changed underneath them. Two directions:
 * - `subscribe`: something *else* changed the data (another device or person, pulled by sync or
 *   announced by the server). Pages re-render. `showId` is null when the list of shows changed.
 * - `onLocalWrite`: this device just edited something. The sync engine pushes soon.
 */
export interface DataChange {
    showId: string | null;
    /** The revision the change brought the show to, when known (lets a page skip its own edits). */
    rev?: number;
}

export class ChangeBus {
    private readonly external = new Set<(change: DataChange) => void>();
    private readonly local = new Set<() => void>();

    subscribe(listener: (change: DataChange) => void): () => void {
        this.external.add(listener);
        return () => this.external.delete(listener);
    }

    emitExternal(showId: string | null, rev?: number): void {
        for (const listener of [...this.external]) listener(rev === undefined ? { showId } : { showId, rev });
    }

    onLocalWrite(listener: () => void): () => void {
        this.local.add(listener);
        return () => this.local.delete(listener);
    }

    emitLocalWrite(): void {
        for (const listener of [...this.local]) listener();
    }
}
