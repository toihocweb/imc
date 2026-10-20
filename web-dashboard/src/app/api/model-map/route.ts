import { NextResponse } from "next/server";
import { readModelMappings, removeModelMapping, setModelMapping } from "@/lib/model-map";

/** The model mappings the proxy applies to incoming requests. */
export async function GET() {
  try {
    return NextResponse.json(readModelMappings());
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}

/** Add or replace one mapping: { from, to }. */
export async function POST(req: Request) {
  let body: any;
  try {
    body = await req.json();
  } catch {
    return NextResponse.json({ error: "invalid JSON body" }, { status: 400 });
  }
  const from = typeof body.from === "string" ? body.from.trim() : "";
  const to = typeof body.to === "string" ? body.to.trim() : "";
  if (!from || !to) {
    return NextResponse.json({ error: "from and to are required" }, { status: 400 });
  }
  try {
    return NextResponse.json(setModelMapping(from, to));
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}

/** Remove one mapping: { from }. */
export async function DELETE(req: Request) {
  let body: any;
  try {
    body = await req.json();
  } catch {
    return NextResponse.json({ error: "invalid JSON body" }, { status: 400 });
  }
  const from = typeof body.from === "string" ? body.from.trim() : "";
  if (!from) {
    return NextResponse.json({ error: "from is required" }, { status: 400 });
  }
  try {
    return NextResponse.json(removeModelMapping(from));
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}
