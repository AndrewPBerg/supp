"""Minimal CLI entry point for the demo project."""

import sys

from models import User, Project
from service import create_user, create_project, admin_users


def main():
    """Run the demo workflow: create users, a project, then list admins."""
    alice = create_user("Alice", "alice@example.com", role="admin")
    bob = create_user("Bob", "bob@example.com")

    project = create_project("supp demo", alice)

    print(f"Created project: {project.title} (owner: {alice.name})")
    print(f"Admins: {[u.name for u in admin_users()]}")

    if not project.validate():
        print("project failed validation", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
