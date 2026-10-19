import json, subprocess
IMC = "/root/imc-rs/target/release/imc"
JSON_ARRAY = json.dumps([
    {"tool": "read", "file": "src/main.rs", "content": "fn main() { println!(\"hello\"); }"},
] * 10)
print(f"JSON_ARRAY length: {len(JSON_ARRAY)} chars")
print(f"est_tokens: {len(JSON_ARRAY) // 4}")
result = subprocess.run([IMC, "compress", "--raw"], input=JSON_ARRAY, capture_output=True, text=True, timeout=60)
print(f"RC: {result.returncode}")
if result.stdout:
    data = json.loads(result.stdout)
    print(f"Rust ratio: {data['compression_ratio']}")
    print(f"Rust saved_tokens: {data['saved_tokens']}")
    print(f"Rust original_tokens: {data['original_tokens']}")
else:
    print("No stdout:", result.stderr[:200])
