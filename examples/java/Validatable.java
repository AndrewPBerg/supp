package examples;

/**
 * Interface for objects that can validate their own state.
 */
public interface Validatable {
    /** Validate this object and return the result. */
    ValidationResult validate();
}
