import { NextResponse } from "next/server";
import { ccrList } from "@/lib/imc";

export async function GET(req: Request) {
  const { searchParams } = new URL(req.url);
  const root = searchParams.get("root") || undefined;
  const id = searchParams.get("id") || undefined;
  const limit = Number(searchParams.get("limit") || "20");
  try {
    const list = await ccrList(root, limit, id);
    return NextResponse.json(list);
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}
