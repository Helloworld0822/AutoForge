import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';
import { getAuthMe, login as apiLogin, logout as apiLogout } from '../api/client';
import type { AuthMeResponse } from '../types';

interface AuthContextValue {
  user: AuthMeResponse | null;
  loading: boolean;
  loginRequired: boolean;
  login: (username: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  refresh: () => Promise<void>;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<AuthMeResponse | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    try {
      const me = await getAuthMe();
      setUser(me);
    } catch {
      setUser({
        authenticated: false,
        username: null,
        session_login_enabled: true,
        api_key_enabled: false,
      });
    }
  }, []);

  useEffect(() => {
    refresh().finally(() => setLoading(false));
  }, [refresh]);

  const login = useCallback(
    async (username: string, password: string) => {
      await apiLogin(username, password);
      await refresh();
    },
    [refresh],
  );

  const logout = useCallback(async () => {
    try {
      await apiLogout();
    } finally {
      setUser((prev) =>
        prev
          ? { ...prev, authenticated: false, username: null }
          : {
              authenticated: false,
              username: null,
              session_login_enabled: true,
              api_key_enabled: false,
            },
      );
    }
  }, []);

  const loginRequired = Boolean(user?.session_login_enabled && !user.authenticated);

  const value = useMemo(
    () => ({ user, loading, loginRequired, login, logout, refresh }),
    [user, loading, loginRequired, login, logout, refresh],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext);
  if (!ctx) {
    throw new Error('useAuth must be used within AuthProvider');
  }
  return ctx;
}
