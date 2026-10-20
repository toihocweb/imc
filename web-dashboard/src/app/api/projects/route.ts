import { NextResponse } from "next/server";
import { listProjects } from "@/lib/imc";

export async function GET(req: Request) {
  const { searchParams } = new URL(req.url);
  const root = searchParams.get("root") || undefined;
  try {
    const projects = await listProjects(root);
    return NextResponse.json(projects);
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}
