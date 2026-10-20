import { NextResponse } from "next/server";
import { trace } from "@/lib/imc";

export async function POST(req: Request) {
  const body = await req.json();
  const { target, root, callers, both, depth } = body;
  if (!target) {
    return NextResponse.json({ error: "target required" }, { status: 400 });
  }
  try {
    const result = await trace(target, {
      root,
      callers,
      both,
      depth,
    });
    return NextResponse.json(result);
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}
