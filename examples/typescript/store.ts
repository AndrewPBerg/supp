import { Entity, EntityId } from "./models";

/**
 * Generic in-memory store with CRUD operations.
 *
 * Backed by a Map for O(1) lookups. Designed to be
 * subclassed for domain-specific validation.
 */
export class Store<T extends Entity> {
  protected items: Map<EntityId, T> = new Map();

  /** Insert an entity. Throws if the ID already exists. */
  add(item: T): void {
    if (this.items.has(item.id)) {
      throw new Error(`duplicate id: ${item.id}`);
    }
    this.items.set(item.id, item);
  }

  /** Retrieve by ID, or undefined if missing. */
  get(id: EntityId): T | undefined {
    return this.items.get(id);
  }

  /** Remove by ID. Returns true if the item existed. */
  delete(id: EntityId): boolean {
    return this.items.delete(id);
  }

  /** Return all stored entities. */
  all(): T[] {
    return Array.from(this.items.values());
  }

  /** Number of stored entities. */
  get size(): number {
    return this.items.size;
  }
}

/**
 * A store that validates items before insertion.
 *
 * Subclasses provide a `validate` method; `add` rejects
 * items that fail validation.
 */
export abstract class ValidatedStore<T extends Entity> extends Store<T> {
  /** Return null if valid, or an error message string. */
  abstract validate(item: T): string | null;

  add(item: T): void {
    const err = this.validate(item);
    if (err) {
      throw new Error(`validation failed: ${err}`);
    }
    super.add(item);
  }
}
