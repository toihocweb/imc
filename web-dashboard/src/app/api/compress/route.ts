import { NextResponse } from "next/server";
import { compressText } from "@/lib/imc";

export async function POST(req: Request) {
  const body = await req.json();
  const { text, root } = body;
  if (!text) {
    return NextResponse.json({ error: "text required" }, { status: 400 });
  }
  try {
    const result = await compressText(text, root);
    return NextResponse.json({ compressed: result });
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}
