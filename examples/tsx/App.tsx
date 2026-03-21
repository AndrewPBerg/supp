import { useEffect, useState } from "react";
import { AppProps, UserCardProps } from "./types";
import { UserCard } from "./UserCard";
import { useAuth } from "./hooks";

/** Top-level app shell that lists users. */
export function App({ title, users }: AppProps) {
  const [auth] = useAuth();
  const [filter, setFilter] = useState("");

  useEffect(() => {
    document.title = title;
  }, [title]);

  const filtered = users.filter((u) =>
    u.name.toLowerCase().includes(filter.toLowerCase())
  );

  return (
    <div className="app">
      <h1>{title}</h1>
      {auth && <p>Logged in as {auth}</p>}
      <input
        placeholder="Filter users..."
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
      />
      {filtered.map((user) => (
        <UserCard key={user.email} {...user} />
      ))}
    </div>
  );
}
