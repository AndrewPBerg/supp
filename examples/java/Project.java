package examples;

import java.util.ArrayList;
import java.util.List;

/**
 * A project containing tasks, owned by a user.
 */
public class Project extends BaseEntity {
    private final String title;
    private final String ownerId;
    private final List<String> tags;

    public Project(String id, String title, String ownerId) {
        super(id);
        this.title = title;
        this.ownerId = ownerId;
        this.tags = new ArrayList<>();
    }

    public String getTitle() { return title; }
    public String getOwnerId() { return ownerId; }
    public List<String> getTags() { return tags; }
}
