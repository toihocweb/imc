import { NextResponse } from "next/server";
import { ccrStats } from "@/lib/imc";

export async function GET(req: Request) {
  const { searchParams } = new URL(req.url);
  const root = searchParams.get("root") || undefined;
  try {
    const stats = await ccrStats(root);
    return NextResponse.json(stats);
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}
