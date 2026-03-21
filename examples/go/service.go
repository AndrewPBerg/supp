package main

import "fmt"

// AppState holds the in-memory store for users and projects.
type AppState struct {
	users    []*User
	projects []*Project
	nextID   int
}

// NewAppState creates an empty application state.
func NewAppState() *AppState {
	return &AppState{nextID: 1}
}

func (s *AppState) genID(prefix string) string {
	id := fmt.Sprintf("%s-%d", prefix, s.nextID)
	s.nextID++
	return id
}

// CreateUser adds a new user to the store.
func (s *AppState) CreateUser(name, email string, role Role) (*User, error) {
	user := &User{
		ID:    s.genID("u"),
		Name:  name,
		Email: email,
		Role:  role,
	}
	if err := user.Validate(); err != nil {
		return nil, err
	}
	s.users = append(s.users, user)
	return user, nil
}

// GetUser looks up a user by ID.
func (s *AppState) GetUser(id string) *User {
	for _, u := range s.users {
		if u.ID == id {
			return u
		}
	}
	return nil
}

// CreateProject creates a project owned by the given user.
func (s *AppState) CreateProject(title string, owner *User) (*Project, error) {
	project := &Project{
		ID:      s.genID("p"),
		Title:   title,
		OwnerID: owner.ID,
	}
	if err := project.Validate(); err != nil {
		return nil, err
	}
	s.projects = append(s.projects, project)
	return project, nil
}

// AdminUsers returns all users with admin privileges.
func (s *AppState) AdminUsers() []*User {
	var admins []*User
	for _, u := range s.users {
		if u.IsAdmin() {
			admins = append(admins, u)
		}
	}
	return admins
}
