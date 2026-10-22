import { NextResponse } from "next/server";
import { memoryList, memoryAdd, memoryDelete } from "@/lib/imc";

export async function GET(req: Request) {
  const { searchParams } = new URL(req.url);
  const root = searchParams.get("root") || undefined;
  try {
    const memories = await memoryList(root);
    return NextResponse.json(memories);
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}

export async function POST(req: Request) {
  const body = await req.json();
  const { root, content, category, memType, scene, path, tag, importance } = body;
  if (!root || !content) {
    return NextResponse.json({ error: "root and content required" }, { status: 400 });
  }
  try {
    const result = await memoryAdd(
      root,
      content,
      category || "note",
      memType || "note",
      scene || "",
      path || [],
      tag || [],
      importance || 0.5
    );
    return NextResponse.json({ success: true, output: result });
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}

export async function DELETE(req: Request) {
  const { searchParams } = new URL(req.url);
  const idStr = searchParams.get("id");
  let root = searchParams.get("root") || undefined;
  let id = idStr ? Number(idStr) : undefined;

  if (!id) {
    try {
      const body = await req.json();
      if (body?.id) id = Number(body.id);
      if (body?.root) root = body.root;
    } catch {}
  }

  if (!id) {
    return NextResponse.json({ error: "id required" }, { status: 400 });
  }

  try {
    const result = await memoryDelete(id, root);
    return NextResponse.json(result);
  } catch (e: any) {
    return NextResponse.json({ error: e.message }, { status: 500 });
  }
}
