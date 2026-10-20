import { NextResponse } from "next/server";
import { search } from "@/lib/imc";

export async function POST(req: Request) {
  const body = await req.json();
  const { query, root, limit, noVector } = body;
  if (!query) {
    return NextResponse.json({ error: "query required" }, { status: 400 });
  }
  try {
    const result = await search(query, {
      root,
      limit,
      noVector,
    });
    return NextResponse.json(result);
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}
