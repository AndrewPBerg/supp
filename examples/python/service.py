"""Business logic layer — orchestrates models and storage."""

from typing import Optional

from models import User, Project, Task


# In-memory store for demo purposes
_users: dict[str, User] = {}
_projects: dict[str, Project] = {}


def create_user(name: str, email: str, role: str = "viewer") -> User:
    """Create a new user and add to the store.

    Args:
        name: Display name.
        email: Must be unique.
        role: One of "viewer", "editor", "admin".

    Returns:
        The newly created User.
    """
    uid = f"u-{len(_users) + 1}"
    user = User(id=uid, name=name, email=email, role=role)
    if not user.validate():
        raise ValueError("invalid user data")
    _users[uid] = user
    return user


def get_user(user_id: str) -> Optional[User]:
    """Look up a user by ID."""
    return _users.get(user_id)


def create_project(title: str, owner: User) -> Project:
    """Create a project owned by the given user.

    Raises ValueError if the owner has no ID.
    """
    if owner.id is None:
        raise ValueError("owner must have an id")
    pid = f"p-{len(_projects) + 1}"
    project = Project(id=pid, title=title, owner_id=owner.id)
    _projects[pid] = project
    return project


def add_task(project: Project, description: str) -> Task:
    """Append a task to a project."""
    tid = f"t-{len(_projects) + 1}"
    task = Task(id=tid, project_id=project.id, description=description)
    return task


def admin_users() -> list[User]:
    """Return all users with admin privileges."""
    return [u for u in _users.values() if u.is_admin()]
