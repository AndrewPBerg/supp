package examples;

import java.util.ArrayList;
import java.util.List;
import java.util.stream.Collectors;

/**
 * Service layer for managing users and projects.
 */
public class ProjectService {
    private final List<User> users = new ArrayList<>();
    private final List<Project> projects = new ArrayList<>();
    private int nextId = 1;

    private String genId(String prefix) {
        return prefix + "-" + (nextId++);
    }

    /**
     * Create and store a new user.
     *
     * @param name  display name
     * @param email must contain '@'
     * @param role  permission level
     * @return the created user
     * @throws IllegalArgumentException if validation fails
     */
    public User createUser(String name, String email, Role role) {
        User user = new User(genId("u"), name, email, role);
        ValidationResult result = user.validate();
        if (!result.isValid()) {
            throw new IllegalArgumentException(result.getMessage());
        }
        users.add(user);
        return user;
    }

    /** Create a project owned by the given user. */
    public Project createProject(String title, User owner) {
        Project project = new Project(genId("p"), title, owner.getId());
        projects.add(project);
        return project;
    }

    /** Return all users with admin privileges. */
    public List<User> adminUsers() {
        return users.stream()
                .filter(User::isAdmin)
                .collect(Collectors.toList());
    }
}
