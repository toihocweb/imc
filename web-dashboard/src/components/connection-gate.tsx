"use client";

import React, { createContext, useContext, useEffect, useState } from "react";
import { Lock, Key, ShieldCheck, Eye, EyeOff, Loader2, AlertCircle, ArrowRight, CheckCircle2, Server } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";

interface ConnectionContextType {
  isConnected: boolean;
  secret: string;
  disconnect: () => void;
  openGate: () => void;
}

const ConnectionContext = createContext<ConnectionContextType>({
  isConnected: false,
  secret: "",
  disconnect: () => {},
  openGate: () => {},
});

export function useConnection() {
  return useContext(ConnectionContext);
}

export function ConnectionGate({ children }: { children: React.ReactNode }) {
  const [mounted, setMounted] = useState(false);
  const [initialChecking, setInitialChecking] = useState(true);
  const [isConnected, setIsConnected] = useState(false);
  const [secret, setSecret] = useState("");
  const [showSecret, setShowSecret] = useState(false);
  const [checking, setChecking] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);

  const verifyConnection = async (keyToTest: string): Promise<boolean> => {
    if (!keyToTest.trim()) {
      setErrorMsg("Please enter your API key / secret.");
      return false;
    }
    setChecking(true);
    setErrorMsg(null);
    setSuccessMsg(null);
    try {
      const res = await fetch("/api/connection/check", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ secret: keyToTest.trim() }),
      });
      const data = await res.json();
      if (res.ok && data.ok) {
        localStorage.setItem("imc_hyperspace_secret", keyToTest.trim());
        setSuccessMsg(data.message || "Connected successfully!");
        setIsConnected(true);
        return true;
      } else {
        setErrorMsg(data.error || "Connection failed. Please ensure Hyperspace is running on port 6655 and your API key is valid.");
        setIsConnected(false);
        return false;
      }
    } catch (err: any) {
      setErrorMsg(`Network error or proxy unreachable: ${err?.message ?? err}`);
      setIsConnected(false);
      return false;
    } finally {
      setChecking(false);
    }
  };

  useEffect(() => {
    setMounted(true);

    const checkInitialConnection = async () => {
      setInitialChecking(true);
      try {
        // 1. Check server configuration first (config.yaml)
        const res = await fetch("/api/connection/check");
        const data = await res.json();
        if (res.ok && data.ok) {
          setIsConnected(true);
          setSuccessMsg(data.message || "Connected successfully via server configuration!");
          return;
        }

        // 2. If server config has no valid connection, check localStorage fallback
        const saved = localStorage.getItem("imc_hyperspace_secret");
        if (saved) {
          setSecret(saved);
          const ok = await verifyConnection(saved);
          if (ok) return;
        }

        // 3. If server reported a specific error with existing key, surface it
        if (data.configured && data.error) {
          setErrorMsg(data.error);
        }
      } catch (err: any) {
        // Fallback to localStorage if GET endpoint error
        const saved = localStorage.getItem("imc_hyperspace_secret");
        if (saved) {
          setSecret(saved);
          verifyConnection(saved);
        }
      } finally {
        setInitialChecking(false);
      }
    };

    checkInitialConnection();
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    await verifyConnection(secret);
  };

  const disconnect = () => {
    localStorage.removeItem("imc_hyperspace_secret");
    setIsConnected(false);
    setSecret("");
    setErrorMsg(null);
    setSuccessMsg(null);
  };

  const openGate = () => {
    setIsConnected(false);
    setErrorMsg(null);
  };

  if (!mounted || initialChecking) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-background">
        <div className="flex flex-col items-center gap-3 text-muted-foreground text-xs">
          <Loader2 className="h-6 w-6 animate-spin text-primary" />
          <span>Verifying Hyperspace connection...</span>
        </div>
      </div>
    );
  }

  if (!isConnected) {
    return (
      <div className="min-h-screen w-full flex items-center justify-center p-4 bg-background relative overflow-hidden">
        {/* Ambient background glow */}
        <div className="absolute top-1/4 left-1/2 -translate-x-1/2 -translate-y-1/2 w-96 h-96 bg-primary/10 rounded-full blur-3xl pointer-events-none" />

        <Card className="w-full max-w-md border-border/80 shadow-xl glow-card relative z-10">
          <CardHeader className="text-center pb-4 pt-6">
            <div className="mx-auto mb-3 flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10 text-primary border border-primary/20">
              <ShieldCheck className="h-6 w-6" />
            </div>
            <CardTitle className="text-xl font-bold tracking-tight text-foreground">
              Hyperspace Connection Gate
            </CardTitle>
            <p className="text-xs text-muted-foreground mt-1.5 leading-relaxed">
              Default connection targets Hyperspace on port 6655 (<code className="font-mono text-[11px] text-foreground">localhost:6655</code>).
              Enter your API key / secret to verify connection before accessing the dashboard.
            </p>
          </CardHeader>

          <CardContent className="space-y-4 pt-2 pb-6 px-6">
            <form onSubmit={handleSubmit} className="space-y-4">
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <label className="text-xs font-semibold text-foreground flex items-center gap-1.5">
                    <Key className="h-3.5 w-3.5 text-primary" /> API Key / Secret
                  </label>
                  <Badge variant="secondary" className="font-mono text-[10px] px-2 py-0.5">
                    Port 6655
                  </Badge>
                </div>
                <div className="relative">
                  <Input
                    type={showSecret ? "text" : "password"}
                    placeholder="Enter secret / API key..."
                    value={secret}
                    onChange={(e) => setSecret(e.target.value)}
                    disabled={checking}
                    className="h-10 text-xs sm:text-sm pr-10 font-mono"
                    autoFocus
                  />
                  <button
                    type="button"
                    onClick={() => setShowSecret(!showSecret)}
                    className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground transition-colors cursor-pointer p-1"
                    title={showSecret ? "Hide" : "Show"}
                  >
                    {showSecret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                  </button>
                </div>
              </div>

              {errorMsg && (
                <div className="rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-xs text-destructive flex items-start gap-2">
                  <AlertCircle className="h-4 w-4 shrink-0 mt-0.5" />
                  <div className="flex-1 leading-relaxed break-words">{errorMsg}</div>
                </div>
              )}

              {successMsg && (
                <div className="rounded-lg border border-emerald-500/40 bg-emerald-500/10 p-3 text-xs text-emerald-500 flex items-start gap-2">
                  <CheckCircle2 className="h-4 w-4 shrink-0 mt-0.5" />
                  <div className="flex-1 leading-relaxed">{successMsg}</div>
                </div>
              )}

              <Button
                type="submit"
                disabled={checking || !secret.trim()}
                className="w-full h-10 text-xs sm:text-sm font-semibold cursor-pointer flex items-center justify-center gap-2 shadow-sm"
              >
                {checking ? (
                  <>
                    <Loader2 className="h-4 w-4 animate-spin" />
                    <span>Verifying connection...</span>
                  </>
                ) : (
                  <>
                    <span>Verify Connection & Enter Dashboard</span>
                    <ArrowRight className="h-4 w-4" />
                  </>
                )}
              </Button>
            </form>

            <div className="pt-2 border-t border-border/60 flex items-center justify-between text-[11px] text-muted-foreground">
              <span className="flex items-center gap-1">
                <Server className="h-3 w-3" /> Target: <code className="font-mono">127.0.0.1:6655</code>
              </span>
              <span>IMC Studio v0.2.2</span>
            </div>
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <ConnectionContext.Provider value={{ isConnected, secret, disconnect, openGate }}>
      {children}
    </ConnectionContext.Provider>
  );
}
