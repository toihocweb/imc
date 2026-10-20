import { NextResponse } from "next/server";
import { spawn } from "child_process";

const IMC = process.env.IMC_BIN || "/root/imc-rs/target/release/imc";

function runImc(args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    const env = { ...process.env };
    delete env.IMC_DATA_DIR;
    delete env.IMC_CONFIG;
    const child = spawn(IMC, args, { env, stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    child.stdout.on("data", (d) => (out += d.toString()));
    child.stderr.on("data", (d) => (err += d.toString()));
    child.on("close", (code) =>
      code === 0 ? resolve(out) : reject(new Error(err || `imc ${args[0]} exited ${code}`))
    );
  });
}

/** List captured prompts + capture state. */
export async function GET(req: Request) {
  const { searchParams } = new URL(req.url);
  const id = searchParams.get("id");
  try {
    if (id) {
      const payload = await runImc(["--json", "capture", "show", id]);
      return NextResponse.json({ ok: true, payload: JSON.parse(payload) });
    }
    const [status, list] = await Promise.all([
      runImc(["--json", "capture", "status"]),
      runImc(["--json", "capture", "list"]),
    ]);
    return NextResponse.json({
      ok: true,
      status: JSON.parse(status),
      captures: JSON.parse(list || "[]"),
    });
  } catch (e: any) {
    return NextResponse.json({ ok: false, error: e.message }, { status: 500 });
  }
}

/** Toggle capture, or delete captures. */
export async function POST(req: Request) {
  const body = await req.json().catch(() => ({}));
  try {
    if (body.action === "on" || body.action === "off") {
      await runImc(["capture", body.action]);
    } else if (body.action === "clear") {
      await runImc(["capture", "clear"]);
    } else if (body.action === "delete" && body.id) {
      await runImc(["capture", "rm", body.id]);
    } else {
      return NextResponse.json({ error: "unknown action" }, { status: 400 });
    }
    const [status, list] = await Promise.all([
      runImc(["--json", "capture", "status"]),
      runImc(["--json", "capture", "list"]),
    ]);
    return NextResponse.json({
      ok: true,
      status: JSON.parse(status),
      captures: JSON.parse(list || "[]"),
    });
  } catch (e: any) {
    return NextResponse.json({ ok: false, error: e.message }, { status: 500 });
  }
}
