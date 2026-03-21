"""Base model layer with serialization support."""

from dataclasses import dataclass, field
from typing import Any, Optional


@dataclass
class BaseModel:
    """Root of the model hierarchy.

    All domain models inherit from this to get
    consistent serialization and validation.
    """

    id: Optional[str] = None

    def validate(self) -> bool:
        """Check required fields are present."""
        return self.id is not None

    def to_dict(self) -> dict[str, Any]:
        """Serialize to a plain dictionary."""
        return {"id": self.id}

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "BaseModel":
        """Deserialize from a dictionary."""
        return cls(id=data.get("id"))


@dataclass
class User(BaseModel):
    """A user account with email and role."""

    name: str = ""
    email: str = ""
    role: str = "viewer"

    def validate(self) -> bool:
        return super().validate() and bool(self.email)

    def to_dict(self) -> dict[str, Any]:
        base = super().to_dict()
        base.update({"name": self.name, "email": self.email, "role": self.role})
        return base

    def is_admin(self) -> bool:
        """Check if the user has admin privileges."""
        return self.role == "admin"


@dataclass
class Project(BaseModel):
    """A project owned by a user."""

    title: str = ""
    owner_id: Optional[str] = None
    tags: list[str] = field(default_factory=list)

    def validate(self) -> bool:
        return super().validate() and bool(self.title)

    def to_dict(self) -> dict[str, Any]:
        base = super().to_dict()
        base.update({"title": self.title, "owner_id": self.owner_id, "tags": self.tags})
        return base


@dataclass
class Task(BaseModel):
    """A task within a project."""

    project_id: Optional[str] = None
    description: str = ""
    done: bool = False

    def toggle(self):
        """Flip the completion status."""
        self.done = not self.done

    def to_dict(self) -> dict[str, Any]:
        base = super().to_dict()
        base.update({
            "project_id": self.project_id,
            "description": self.description,
            "done": self.done,
        })
        return base
