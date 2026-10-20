import { NextResponse } from "next/server";
import { configSet, fetchModels } from "@/lib/imc";

/**
 * Save the URL/key the user just typed, then ask the endpoint for its model
 * list. The key is written to the config file and never sent back.
 */
export async function POST(req: Request) {
  let body: any = {};
  try {
    body = await req.json();
  } catch {
    body = {};
  }

  try {
    if (typeof body.base_url === "string" && body.base_url.trim()) {
      await configSet("llm.base_url", body.base_url.trim());
    }
    if (typeof body.api_key === "string" && body.api_key.trim()) {
      await configSet("llm.api_key", body.api_key.trim());
    }
  } catch (e: any) {
    return NextResponse.json({ error: `could not save: ${e.message}` }, { status: 400 });
  }

  try {
    const list = await fetchModels();
    return NextResponse.json({ ok: true, ...list });
  } catch (e: any) {
    return NextResponse.json({ ok: false, error: e.message, models: [] }, { status: 502 });
  }
}
