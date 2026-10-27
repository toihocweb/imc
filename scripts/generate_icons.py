#!/usr/bin/env python3
"""
Generate comprehensive icon suite for IMC (IndexMyCode).
Produces:
  - SVG master and app icons
  - Multi-size favicon.ico (16, 32, 48)
  - Standard web and PWA PNG icons (16, 32, 180, 192, 512)
  - site.webmanifest
"""

import math
from pathlib import Path
from PIL import Image, ImageDraw

SVG_CONTENT = '''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512" fill="none">
  <defs>
    <radialGradient id="glow" cx="50%" cy="50%" r="60%">
      <stop offset="0%" stop-color="#6366F1" stop-opacity="0.25" />
      <stop offset="100%" stop-color="#0A0F1D" stop-opacity="0" />
    </radialGradient>
    <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#0F172A" />
      <stop offset="50%" stop-color="#0B0F1D" />
      <stop offset="100%" stop-color="#050814" />
    </linearGradient>
    <linearGradient id="border" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#38BDF8" stop-opacity="0.8" />
      <stop offset="50%" stop-color="#6366F1" stop-opacity="0.5" />
      <stop offset="100%" stop-color="#A855F7" stop-opacity="0.7" />
    </linearGradient>
    <linearGradient id="leftGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#38BDF8" />
      <stop offset="100%" stop-color="#06B6D4" />
    </linearGradient>
    <linearGradient id="rightGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#A78BFA" />
      <stop offset="100%" stop-color="#6366F1" />
    </linearGradient>
    <linearGradient id="spineGrad" x1="0%" y1="0%" x2="0%" y2="100%">
      <stop offset="0%" stop-color="#FFFFFF" />
      <stop offset="50%" stop-color="#E2E8F0" />
      <stop offset="100%" stop-color="#94A3B8" />
    </linearGradient>
    <linearGradient id="dotTop" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#38BDF8" />
      <stop offset="100%" stop-color="#0284C7" />
    </linearGradient>
    <linearGradient id="dotBot" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#818CF8" />
      <stop offset="100%" stop-color="#4F46E5" />
    </linearGradient>
    <filter id="dropGlow" x="-20%" y="-20%" width="140%" height="140%">
      <feGaussianBlur stdDeviation="8" result="blur" />
      <feComposite in="SourceGraphic" in2="blur" operator="over" />
    </filter>
  </defs>

  <!-- Container Squircle -->
  <rect x="18" y="18" width="476" height="476" rx="108" fill="url(#bg)" stroke="url(#border)" stroke-width="12" />
  <rect x="18" y="18" width="476" height="476" rx="108" fill="url(#glow)" />

  <!-- AST & Vector Connection Graph Lines -->
  <g opacity="0.35">
    <line x1="115" y1="256" x2="256" y2="120" stroke="#38BDF8" stroke-width="4" stroke-dasharray="6 6" />
    <line x1="397" y1="256" x2="256" y2="120" stroke="#818CF8" stroke-width="4" stroke-dasharray="6 6" />
    <line x1="115" y1="256" x2="256" y2="392" stroke="#06B6D4" stroke-width="4" stroke-dasharray="6 6" />
    <line x1="397" y1="256" x2="256" y2="392" stroke="#6366F1" stroke-width="4" stroke-dasharray="6 6" />
  </g>

  <!-- Left Code Chevron < -->
  <path d="M 185 165 L 115 256 L 185 347" stroke="url(#leftGrad)" stroke-width="38" stroke-linecap="round" stroke-linejoin="round" filter="url(#dropGlow)" />

  <!-- Center Index Spine 'I' -->
  <rect x="237" y="165" width="38" height="182" rx="19" fill="url(#spineGrad)" filter="url(#dropGlow)" />
  <circle cx="256" cy="120" r="26" fill="url(#dotTop)" filter="url(#dropGlow)" />
  <circle cx="256" cy="392" r="26" fill="url(#dotBot)" filter="url(#dropGlow)" />

  <!-- Right Code Chevron > -->
  <path d="M 327 165 L 397 256 L 327 347" stroke="url(#rightGrad)" stroke-width="38" stroke-linecap="round" stroke-linejoin="round" filter="url(#dropGlow)" />
</svg>'''

MANIFEST_CONTENT = '''{
  "name": "IMC - IndexMyCode",
  "short_name": "IMC",
  "description": "Code intelligence dashboard for coding agents",
  "start_url": "/",
  "display": "standalone",
  "background_color": "#0A0F1A",
  "theme_color": "#0A0F1A",
  "icons": [
    {
      "src": "/favicon-16x16.png",
      "sizes": "16x16",
      "type": "image/png"
    },
    {
      "src": "/favicon-32x32.png",
      "sizes": "32x32",
      "type": "image/png"
    },
    {
      "src": "/apple-touch-icon.png",
      "sizes": "180x180",
      "type": "image/png"
    },
    {
      "src": "/android-chrome-192x192.png",
      "sizes": "192x192",
      "type": "image/png"
    },
    {
      "src": "/android-chrome-512x512.png",
      "sizes": "512x512",
      "type": "image/png"
    },
    {
      "src": "/icon.svg",
      "sizes": "any",
      "type": "image/svg+xml"
    }
  ]
}
'''


def render_raster_icon(target_size: int, scale: int = 4) -> Image.Image:
    render_size = target_size * scale
    img = Image.new("RGBA", (render_size, render_size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    margin = int(render_size * 0.035)
    radius = int(render_size * 0.22)

    # 1. Base squircle
    draw.rounded_rectangle(
        [margin, margin, render_size - margin, render_size - margin],
        radius=radius,
        fill=(10, 15, 29, 255),
    )

    # 2. Top ambient glow for high-res
    if target_size >= 48:
        glow = Image.new("RGBA", (render_size, render_size), (0, 0, 0, 0))
        glow_draw = ImageDraw.Draw(glow)
        glow_draw.ellipse(
            [render_size * 0.15, render_size * 0.05, render_size * 0.85, render_size * 0.65],
            fill=(99, 102, 241, 50),
        )
        img = Image.alpha_composite(img, glow)
        draw = ImageDraw.Draw(img)

    # 3. Border stroke
    b_width = max(int(render_size * 0.024), 2 * scale)
    draw.rounded_rectangle(
        [margin, margin, render_size - margin, render_size - margin],
        radius=radius,
        outline=(56, 189, 248, 190),
        width=b_width,
    )

    cx, cy = render_size / 2.0, render_size / 2.0
    p_left = (int(render_size * 0.225), int(cy))
    p_right = (int(render_size * 0.775), int(cy))
    p_top = (int(cx), int(render_size * 0.235))
    p_bot = (int(cx), int(render_size * 0.765))

    # 4. Connection dashed lines (only for larger sizes to avoid blur at 16/32)
    if target_size >= 64:
        def draw_dashed(p1, p2, color, w, dash_len=18 * scale, gap_len=14 * scale):
            x1, y1 = p1
            x2, y2 = p2
            dist = math.hypot(x2 - x1, y2 - y1)
            steps = int(dist / (dash_len + gap_len))
            if steps == 0:
                return
            dx = (x2 - x1) / dist
            dy = (y2 - y1) / dist
            for i in range(steps):
                s_d = i * (dash_len + gap_len)
                e_d = min(s_d + dash_len, dist)
                draw.line(
                    [(x1 + dx * s_d, y1 + dy * s_d), (x1 + dx * e_d, y1 + dy * e_d)],
                    fill=color,
                    width=w,
                )

        line_w = max(int(render_size * 0.008), 2)
        draw_dashed(p_left, p_top, (56, 189, 248, 90), line_w)
        draw_dashed(p_right, p_top, (129, 140, 248, 90), line_w)
        draw_dashed(p_left, p_bot, (6, 182, 212, 90), line_w)
        draw_dashed(p_right, p_bot, (99, 102, 241, 90), line_w)

    # 5. Left chevron <
    w_ch = max(int(render_size * 0.076), 3 * scale)
    c_left_top = (int(render_size * 0.36), int(cy - render_size * 0.18))
    c_left_tip = (int(render_size * 0.225), int(cy))
    c_left_bot = (int(render_size * 0.36), int(cy + render_size * 0.18))
    draw.line([c_left_top, c_left_tip, c_left_bot], fill=(6, 182, 212, 255), width=w_ch, joint="round")

    # 6. Right chevron >
    c_right_top = (int(render_size * 0.64), int(cy - render_size * 0.18))
    c_right_tip = (int(render_size * 0.775), int(cy))
    c_right_bot = (int(render_size * 0.64), int(cy + render_size * 0.18))
    draw.line([c_right_top, c_right_tip, c_right_bot], fill=(99, 102, 241, 255), width=w_ch, joint="round")

    # 7. Center spine I
    spine_w = max(int(render_size * 0.076), 3 * scale)
    spine_h = int(render_size * 0.36)
    draw.rounded_rectangle(
        [cx - spine_w / 2, cy - spine_h / 2, cx + spine_w / 2, cy + spine_h / 2],
        radius=int(spine_w / 2),
        fill=(248, 250, 252, 255),
    )

    # 8. Intelligence dots
    dot_r = max(int(render_size * 0.052), 2 * scale)
    draw.ellipse([cx - dot_r, p_top[1] - dot_r, cx + dot_r, p_top[1] + dot_r], fill=(56, 189, 248, 255))
    draw.ellipse([cx - dot_r, p_bot[1] - dot_r, cx + dot_r, p_bot[1] + dot_r], fill=(129, 140, 248, 255))

    # Downsample using high-quality Lanczos filter
    return img.resize((target_size, target_size), Image.Resampling.LANCZOS)


def main():
    repo_root = Path(__file__).resolve().parent.parent
    web_dir = repo_root / "web-dashboard"
    public_dir = web_dir / "public"
    app_dir = web_dir / "src" / "app"

    public_dir.mkdir(parents=True, exist_ok=True)
    app_dir.mkdir(parents=True, exist_ok=True)

    # 1. Clean old default Vercel templates in public
    for stale in ["file.svg", "globe.svg", "next.svg", "vercel.svg", "window.svg"]:
        stale_file = public_dir / stale
        if stale_file.is_file():
            stale_file.unlink()
            print(f"Removed stale template: {stale}")

    # 2. Write SVG icons
    (public_dir / "imc-logo.svg").write_text(SVG_CONTENT)
    (public_dir / "icon.svg").write_text(SVG_CONTENT)
    (app_dir / "icon.svg").write_text(SVG_CONTENT)
    print("Wrote SVG icons to public/ and src/app/")

    # 3. Write PWA Webmanifest
    (public_dir / "site.webmanifest").write_text(MANIFEST_CONTENT)
    print("Wrote site.webmanifest")

    # 4. Generate PNG sizes
    sizes = {
        "favicon-16x16.png": 16,
        "favicon-32x32.png": 32,
        "apple-touch-icon.png": 180,
        "android-chrome-192x192.png": 192,
        "android-chrome-512x512.png": 512,
    }
    for filename, px in sizes.items():
        img = render_raster_icon(px)
        img.save(public_dir / filename, format="PNG")
        print(f"Generated {filename} ({px}x{px})")

    # 5. Generate Multi-size favicon.ico (16, 32, 48)
    ico_img = render_raster_icon(48)
    ico_img.save(
        public_dir / "favicon.ico",
        format="ICO",
        sizes=[(16, 16), (32, 32), (48, 48)],
    )
    # Also save to src/app/favicon.ico for Next.js App Router root discovery
    ico_img.save(
        app_dir / "favicon.ico",
        format="ICO",
        sizes=[(16, 16), (32, 32), (48, 48)],
    )
    print("Generated multi-size favicon.ico (16, 32, 48) in public/ and src/app/")


if __name__ == "__main__":
    main()
