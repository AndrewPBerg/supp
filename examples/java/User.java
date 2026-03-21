package examples;

/**
 * A user account with role-based permissions.
 *
 * <p>Extends {@link BaseEntity} for common ID and timestamp fields.
 * Implements {@link Validatable} so the service layer can check
 * invariants before persisting.</p>
 */
public class User extends BaseEntity implements Validatable {
    private String name;
    private String email;
    private Role role;

    public User(String id, String name, String email, Role role) {
        super(id);
        this.name = name;
        this.email = email;
        this.role = role;
    }

    @Override
    public ValidationResult validate() {
        if (email == null || !email.contains("@")) {
            return ValidationResult.error("invalid email");
        }
        return ValidationResult.ok();
    }

    /** Check if the user has admin privileges. */
    public boolean isAdmin() {
        return role == Role.ADMIN;
    }

    public String getName() { return name; }
    public String getEmail() { return email; }
    public Role getRole() { return role; }
}
