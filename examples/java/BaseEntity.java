package examples;

import java.time.Instant;

/**
 * Base class for all storable entities.
 *
 * Provides a unique ID and creation timestamp.
 */
public abstract class BaseEntity {
    protected final String id;
    protected final Instant createdAt;

    public BaseEntity(String id) {
        this.id = id;
        this.createdAt = Instant.now();
    }

    public String getId() { return id; }
    public Instant getCreatedAt() { return createdAt; }
}
