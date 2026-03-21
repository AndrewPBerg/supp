import { useState, useEffect } from "react";

/** Track the logged-in user. Returns [user, setUser]. */
export function useAuth() {
  const [user, setUser] = useState<string | null>(null);

  useEffect(() => {
    const stored = localStorage.getItem("user");
    if (stored) setUser(stored);
  }, []);

  return [user, setUser] as const;
}

/** Debounce a value by the given delay (ms). */
export function useDebounce<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value);

  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);

  return debounced;
}
