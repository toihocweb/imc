import { NextResponse } from "next/server";

/**
 * The captured prompts and the capture state.
 *
 * The data comes from the imc proxy — the same surface the agents use — not
 * from a subprocess: the proxy is already running and already owns the
 * capture store.
 */
const PROXY_URL = (
  process.env.IMC_PROXY_URL || "http://127.0.0.1:12345"
).replace(/\/$/, "");

async function proxy<T = unknown>(
  path: string,
  init?: RequestInit
): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${PROXY_URL}${path}`, {
      ...init,
      headers: {
        "content-type": "application/json",
        ...(init?.headers || {}),
      },
    });
  } catch (e: any) {
    throw new Error(
      `the imc proxy is not answering on ${PROXY_URL} — start it with \`imc proxy serve --port 12345\` (${e?.message ?? e})`
    );
  }
  const text = await res.text();
  let body: any = null;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    body = text;
  }
  if (!res.ok) {
    throw new Error(
      (body && typeof body === "object" && body.error) ||
        `the imc proxy answered ${res.status}`
    );
  }
  return body as T;
}

/** List captured prompts + capture state. */
export async function GET(req: Request) {
  const { searchParams } = new URL(req.url);
  const id = searchParams.get("id");
  try {
    const q = id ? `?id=${encodeURIComponent(id)}` : "";
    const result = await proxy<{ ok: boolean; payload?: unknown }>(
      `/api/captures${q}`
    );
    return NextResponse.json(result);
  } catch (e: any) {
    return NextResponse.json({ ok: false, error: e.message }, { status: 500 });
  }
}

/** Toggle capture, or delete captures. */
export async function POST(req: Request) {
  const body = await req.json().catch(() => ({}));
  try {
    const result = await proxy<{ ok: boolean }>("/api/captures", {
      method: "POST",
      body: JSON.stringify(body),
    });
    return NextResponse.json(result);
  } catch (e: any) {
    return NextResponse.json({ ok: false, error: e.message }, { status: 500 });
  }
}
