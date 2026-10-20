import { NextResponse } from "next/server";
import { configSet, configStatus, configTest } from "@/lib/imc";

/** Read the effective config (the CLI masks the key, so it never leaves masked). */
export async function GET() {
  try {
    return NextResponse.json(await configStatus());
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}

/** Write config keys and optionally probe the endpoint. */
export async function POST(req: Request) {
  let body: any;
  try {
    body = await req.json();
  } catch {
    return NextResponse.json({ error: "invalid JSON body" }, { status: 400 });
  }

  const applied: string[] = [];
  const errors: string[] = [];

  const writes: [string, string | undefined][] = [
    ["llm.base_url", body.base_url],
    ["llm.api_key", body.api_key],
    ["llm.model", body.model],
    ["embedding.model", body.embedding_model],
  ];

  for (const [key, value] of writes) {
    if (typeof value !== "string" || value.trim() === "") continue;
    try {
      await configSet(key, value.trim());
      applied.push(key);
    } catch (e: any) {
      errors.push(`${key}: ${e.message}`);
    }
  }

  let test: string | null = null;
  if (body.test) {
    try {
      test = await configTest();
    } catch (e: any) {
      test = `ERROR: ${e.message}`;
    }
  }

  try {
    const status = await configStatus();
    return NextResponse.json({ ok: errors.length === 0, applied, errors, test, status });
  } catch (e: any) {
    return NextResponse.json({ ok: false, applied, errors, error: e.message }, { status: 500 });
  }
}
