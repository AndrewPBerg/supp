package main

// Role represents a permission level.
type Role string

const (
	RoleViewer Role = "viewer"
	RoleEditor Role = "editor"
	RoleAdmin  Role = "admin"
)

// Validator is implemented by types that can check their own state.
type Validator interface {
	Validate() error
}

// User represents an account in the system.
type User struct {
	ID    string
	Name  string
	Email string
	Role  Role
}

// Validate checks that the user has a valid email.
func (u *User) Validate() error {
	if u.Email == "" {
		return fmt.Errorf("email is required")
	}
	return nil
}

// IsAdmin returns true if the user has admin privileges.
func (u *User) IsAdmin() bool {
	return u.Role == RoleAdmin
}

// Project is a collection of tasks owned by a user.
type Project struct {
	ID      string
	Title   string
	OwnerID string
	Tags    []string
}

// Validate checks that the project has a title.
func (p *Project) Validate() error {
	if p.Title == "" {
		return fmt.Errorf("title is required")
	}
	return nil
}

// Task is a single work item within a project.
type Task struct {
	ID          string
	ProjectID   string
	Description string
	Done        bool
}

// Toggle flips the completion status of the task.
func (t *Task) Toggle() {
	t.Done = !t.Done
}
