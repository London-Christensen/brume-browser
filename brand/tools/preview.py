#!/usr/bin/env python3
"""Generate preview.html for the Brume kit. Self-contained; SVG comes from kit.py."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kit as K

ROOT = K.ROOT
A, B = K.CLEAVE
ASM, BSM = K.CLEAVE_SM


def m(px, small=False, cls=""):
    ps = K.CLEAVE_SM if small else K.CLEAVE
    return (f'<svg class="{cls}" width="{px}" height="{px}" viewBox="0 0 32 32" '
            f'fill="currentColor" aria-hidden="true"><path d="{" ".join(ps)}"/></svg>')


def lk(kind="h", cls="lk"):
    s = K.lockup_h() if kind == "h" else K.lockup_v()
    return s.replace('<svg xmlns="http://www.w3.org/2000/svg"', f'<svg class="{cls}"')


def wm(cls="wm"):
    return K.wordmark().replace('<svg xmlns="http://www.w3.org/2000/svg"', f'<svg class="{cls}"')


def ic(name, px=40):
    return (K.icon(name)
            .replace('<svg xmlns="http://www.w3.org/2000/svg"', f'<svg width="{px}" height="{px}"'))


ICONSET = "".join(
    f'<div class="ico{" brandico" if n == "cleave" else ""}">{ic(n, 26)}'
    f'<span class="nm">{n}</span></div>' for n in __import__("icons").ALL)

LADDER = "".join(f'<div class="rung">{m(px, px <= 24)}'
                 f'<span class="px{" rd" if px <= 24 else ""}">{px}</span></div>'
                 for px in (128, 96, 64, 48, 32, 24, 16))

SWATCH = [("Haar", K.HAAR, "74 92 107", "31 14 0 58", "primary"),
          ("Ink", K.INK, "16 20 24", "33 17 0 91", "neutral"),
          ("Paper", K.PAPER, "243 244 245", "1 0 0 4", "neutral"),
          ("Lamplight", K.LAMPLIGHT, "198 168 124", "0 15 37 22", "accent")]
SWATCHES = "".join(
    f'<div class="sw"><div class="chip" style="background:{hx}'
    f'{";box-shadow:inset 0 0 0 1px #DEE1E4" if nm == "Paper" else ""}"></div>'
    f'<div class="meta"><b>{nm}</b>{hx}<br>{rgb}<br>{cmyk}<br>'
    f'<i>{role}</i></div></div>' for nm, hx, rgb, cmyk, role in SWATCH)

HTML = f'''<!DOCTYPE html>
<html lang="en" data-theme="light"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Brume Brand Kit</title>
<link rel="icon" href="assets/svg/generated/favicon.svg">
<style>
@import url('https://fonts.googleapis.com/css2?family=Archivo:wght@300;400;500;600&family=IBM+Plex+Mono:wght@400&display=swap');
:root{{--haar:{K.HAAR};--haar-d:{K.HAAR_DARK};--ink:{K.INK};--paper:{K.PAPER};--lamp:{K.LAMPLIGHT};
 --sans:"Archivo",system-ui,-apple-system,"Segoe UI",sans-serif;
 --mono:"IBM Plex Mono",ui-monospace,Consolas,monospace}}
html[data-theme=light]{{--bg:var(--paper);--fg:var(--ink);--mut:var(--haar);--rule:#DEE1E4;--sunk:#E9EBED}}
html[data-theme=dark]{{--bg:var(--ink);--fg:var(--paper);--mut:var(--haar-d);--rule:#232A30;--sunk:#171C21}}
*{{box-sizing:border-box}}
body{{margin:0;background:var(--bg);color:var(--fg);font-family:var(--sans);font-size:16px;
 line-height:26px;letter-spacing:-.005em;-webkit-font-smoothing:antialiased;
 transition:background .25s,color .25s}}
.wrap{{max-width:920px;margin:0 auto;padding:0 32px 140px}}
@media(max-width:640px){{.wrap{{padding:0 20px 80px}}}}

.mast{{display:flex;justify-content:space-between;align-items:flex-start;gap:24px;padding:72px 0 0}}
.mast .lk{{width:210px;height:auto;color:var(--fg);flex:0 0 auto}}
.tg{{font-family:var(--mono);font-size:11px;letter-spacing:.08em;text-transform:uppercase;
 background:none;border:1px solid var(--rule);color:var(--mut);padding:8px 14px;
 border-radius:3px;cursor:pointer;white-space:nowrap}}
.tg:hover{{color:var(--fg);border-color:var(--mut)}}
.tg:focus-visible{{outline:2px solid var(--lamp);outline-offset:2px}}
h1{{font-weight:300;font-size:40px;line-height:44px;letter-spacing:-.03em;margin:52px 0 0}}
.stand{{color:var(--mut);max-width:56ch;margin:14px 0 0}}

section{{padding-top:88px}}
.eye{{font-family:var(--mono);font-size:11px;letter-spacing:.1em;text-transform:uppercase;
 color:var(--mut);margin:0 0 10px;display:flex;gap:14px}}
.eye b{{font-weight:400;color:var(--lamp)}}
h2{{font-weight:500;font-size:20px;line-height:28px;letter-spacing:-.02em;margin:0 0 8px}}
.note{{color:var(--mut);max-width:62ch;margin:0 0 28px}}
code{{font-family:var(--mono);font-size:12.5px;color:var(--mut)}}

/* ---- construction rig ---- */
.rig{{border:1px solid var(--rule);border-radius:4px;padding:36px 32px;display:grid;
 grid-template-columns:280px 1fr;gap:40px;align-items:center}}
@media(max-width:640px){{.rig{{grid-template-columns:1fr;gap:26px}}}}
.rig svg{{width:240px;height:240px;display:block;margin:0 auto}}
.pc{{transition:transform .55s cubic-bezier(.4,0,.2,1),fill .55s,opacity .55s;
 transform-box:view-box;transform-origin:16px 16px;fill:var(--fg)}}
[data-state="apart"] #pA{{transform:translate(-5px,5px)}}
[data-state="apart"] #pB{{transform:translate(5px,-5px)}}
[data-state="proof"] #pA{{fill:var(--haar)}}
[data-state="proof"] #pB{{transform:rotate(180deg);fill:var(--lamp);opacity:.62}}
html[data-theme=dark] [data-state="proof"] #pA{{fill:var(--haar-d)}}
.steps{{display:flex;gap:8px;margin:0 0 18px;flex-wrap:wrap}}
.steps button{{font-family:var(--mono);font-size:11px;letter-spacing:.04em;background:none;
 border:1px solid var(--rule);color:var(--mut);padding:7px 13px;border-radius:3px;cursor:pointer}}
.steps button[aria-pressed=true]{{border-color:var(--lamp);color:var(--fg)}}
.steps button:focus-visible{{outline:2px solid var(--lamp);outline-offset:2px}}
.rigtext{{font-size:15px;line-height:25px;margin:0;min-height:100px}}

.sys{{border:1px solid var(--rule);border-radius:4px;padding:24px 26px;margin-top:14px;
 display:grid;grid-template-columns:repeat(3,1fr);gap:26px}}
@media(max-width:640px){{.sys{{grid-template-columns:1fr;gap:16px}}}}
.sys b{{display:block;font-family:var(--mono);font-size:24px;font-weight:400;line-height:30px}}
.sys span{{font-size:13.5px;line-height:21px;color:var(--mut)}}

/* ---- boards ---- */
.boards{{display:grid;grid-template-columns:1fr 1fr;gap:14px}}
@media(max-width:640px){{.boards{{grid-template-columns:1fr}}}}
.board{{border:1px solid var(--rule);border-radius:4px;padding:34px 28px;display:flex;
 align-items:center;justify-content:center;min-height:132px}}
.board.light{{background:var(--paper);color:var(--ink);border-color:#DEE1E4}}
.board.dark{{background:var(--ink);color:var(--paper);border-color:#232A30}}
.board .lk{{width:100%;max-width:250px;height:auto}}
.board .lkv{{width:auto;height:104px}}
.board .wm{{width:100%;max-width:210px;height:auto}}
.cap{{font-family:var(--mono);font-size:11.5px;line-height:18px;color:var(--mut);margin:10px 0 0;
 display:flex;justify-content:space-between;gap:16px}}
.cap span:last-child{{text-align:right;opacity:.72}}
.stack{{margin-bottom:34px}}

.ladder{{border:1px solid var(--rule);border-radius:4px;display:flex;align-items:flex-end;
 gap:30px;flex-wrap:wrap;padding:38px 32px 26px;color:var(--fg)}}
.rung{{display:flex;flex-direction:column;align-items:center;gap:12px}}
.px{{font-family:var(--mono);font-size:10.5px;color:var(--mut)}}
.px.rd{{color:var(--lamp)}}

.swatches{{display:grid;grid-template-columns:repeat(4,1fr);gap:14px}}
@media(max-width:760px){{.swatches{{grid-template-columns:repeat(2,1fr)}}}}
.sw{{border:1px solid var(--rule);border-radius:4px;overflow:hidden}}
.sw .chip{{height:100px}}
.sw .meta{{padding:14px;font-family:var(--mono);font-size:11px;line-height:18px;color:var(--mut)}}
.sw .meta b{{display:block;font-family:var(--sans);font-size:14px;font-weight:500;
 color:var(--fg);letter-spacing:-.01em;margin-bottom:6px}}
.sw .meta i{{font-style:normal;color:var(--lamp)}}
table{{width:100%;border-collapse:collapse;margin-top:22px;font-size:13.5px}}
th,td{{text-align:left;padding:9px 12px 9px 0;border-bottom:1px solid var(--rule)}}
th{{font-family:var(--mono);font-size:10.5px;letter-spacing:.08em;text-transform:uppercase;
 color:var(--mut);font-weight:400}}
td.n{{font-family:var(--mono);font-size:12.5px}}
.pass{{color:var(--haar)}} html[data-theme=dark] .pass{{color:var(--haar-d)}}
.warn{{color:var(--lamp)}}

.spec{{border:1px solid var(--rule);border-radius:4px;padding:34px 32px}}
.spec .row{{display:grid;grid-template-columns:120px 1fr;gap:24px;padding:20px 0;
 border-bottom:1px solid var(--rule);align-items:baseline}}
.spec .row:first-child{{padding-top:0}} .spec .row:last-child{{border-bottom:0;padding-bottom:0}}
@media(max-width:640px){{.spec .row{{grid-template-columns:1fr;gap:8px}}}}
.lab{{font-family:var(--mono);font-size:10.5px;letter-spacing:.08em;text-transform:uppercase;color:var(--mut)}}
.d1{{font-weight:300;font-size:40px;line-height:44px;letter-spacing:-.03em}}
.d2{{font-weight:400;font-size:28px;line-height:34px;letter-spacing:-.025em}}
.d3{{font-weight:500;font-size:20px;line-height:28px;letter-spacing:-.02em}}
.d4{{font-size:16px;line-height:26px;letter-spacing:-.005em}}
.d5{{font-size:13px;line-height:20px;letter-spacing:.005em;color:var(--mut)}}

.iconset{{display:grid;grid-template-columns:repeat(auto-fill,minmax(104px,1fr));
 gap:1px;background:var(--rule);border:1px solid var(--rule);border-radius:4px;overflow:hidden}}
.ico{{background:var(--bg);display:flex;flex-direction:column;align-items:center;
 justify-content:center;gap:10px;padding:20px 8px;color:var(--fg);min-height:98px}}
.ico .nm{{font-family:var(--mono);font-size:9.5px;color:var(--mut);letter-spacing:.02em;
 text-align:center;line-height:13px}}
.brandico{{color:var(--lamp)}} .brandico .nm{{color:var(--lamp)}}

.mock{{border:1px solid var(--rule);border-radius:4px;overflow:hidden}}
.chrome{{background:var(--ink);padding:10px 12px 0}}
.tabs{{display:flex;gap:4px}}
.tab{{display:flex;align-items:center;gap:9px;padding:9px 14px;border-radius:6px 6px 0 0;
 font-size:12.5px;color:#9DB2C0;background:#171C21;max-width:190px}}
.tab.on{{background:#20272E;color:var(--paper)}}
.bar{{background:#20272E;padding:10px 12px;display:flex;align-items:center;gap:10px}}
.url{{flex:1;background:#101418;border-radius:4px;padding:6px 12px;font-family:var(--mono);
 font-size:12px;color:#9DB2C0}}
.page{{background:var(--paper);height:158px;display:flex;flex-direction:column;
 align-items:center;justify-content:center;gap:22px}}
.page .wmk{{color:var(--haar);opacity:.42}}
.page .field{{width:60%;height:38px;border:1px solid #DEE1E4;border-radius:3px;background:#fff}}
.readme{{background:var(--paper);color:var(--ink);padding:44px 32px 34px;text-align:center}}
.readme .tag{{color:var(--haar);font-size:14.5px;margin:20px 0 16px}}
.readme .badges{{display:flex;gap:6px;justify-content:center;flex-wrap:wrap}}
.readme .badge{{font-family:var(--mono);font-size:10px;background:#E4E6E8;color:#4A5C6B;
 padding:3px 9px;border-radius:3px}}
.dock{{background:var(--ink);padding:34px;display:flex;align-items:flex-end;justify-content:center;gap:22px}}

.rules{{display:grid;grid-template-columns:1fr 1fr;gap:14px}}
@media(max-width:640px){{.rules{{grid-template-columns:1fr}}}}
.rules div{{border:1px solid var(--rule);border-radius:4px;padding:24px 26px}}
.rules h3{{margin:0 0 12px;font-size:13px;font-weight:500;letter-spacing:.04em;text-transform:uppercase}}
.rules .do h3{{color:var(--mut)}} .rules .dont h3{{color:var(--lamp)}}
.rules ul{{margin:0;padding-left:18px}} .rules li{{margin-bottom:9px;font-size:14.5px;line-height:23px}}
.rules li:last-child{{margin-bottom:0}}
footer{{margin-top:96px;padding-top:26px;border-top:1px solid var(--rule);display:flex;
 align-items:center;justify-content:space-between;gap:20px}}
footer p{{font-family:var(--mono);font-size:11px;color:var(--mut);margin:0;text-align:right}}
@media(prefers-reduced-motion:reduce){{*{{transition:none!important}}}}
</style></head><body><div class="wrap">

<div class="mast">{lk("h")}<button class="tg" id="tg">Dark</button></div>
<h1>Brand kit</h1>
<p class="stand">Every asset at true size, light and dark. Section numbers match
BRAND-KIT.md. Start with the construction below: it is the one thing worth
understanding before using anything else here.</p>

<section>
  <p class="eye"><b>&sect; 2.1</b> Construction</p>
  <h2>One square, one cut</h2>
  <p class="note">A stepped cut runs down, across, then down again. Each piece is
  pulled back half a module from that line, opening a 3-unit gap. Step through it.</p>
  <div class="rig" id="rig" data-state="assembled">
    <svg viewBox="-8 -8 48 48" aria-hidden="true">
      <path class="pc" id="pA" d="{A}"/>
      <path class="pc" id="pB" d="{B}"/>
    </svg>
    <div>
      <div class="steps" role="group" aria-label="Construction steps">
        <button data-s="assembled" aria-pressed="true">1 &nbsp;Assembled</button>
        <button data-s="apart" aria-pressed="false">2 &nbsp;Opened</button>
        <button data-s="proof" aria-pressed="false">3 &nbsp;Congruence</button>
      </div>
      <p class="rigtext" id="rigtext">The mark as it ships. Reads as one solid form
      with a hairline running through it. At a glance it is a square; the second
      look is the cut.</p>
    </div>
  </div>
  <div class="sys">
    <div><b>3</b><span>The module. Archivo Medium's stem weight, 0.088em, measured
    off the &lsquo;l&rsquo;. The cut is exactly one module wide.</span></div>
    <div><b>9</b><span>The radius. Half Archivo's bowl, 0.55em, off the &lsquo;o&rsquo;.
    Cleave has no curves, but anything added later stays commensurate.</span></div>
    <div><b>24</b><span>The live area inside a 32 canvas. Equals the ascender height,
    so mark and wordmark are the same height in every lockup.</span></div>
  </div>
</section>

<section>
  <p class="eye"><b>&sect; 2.2</b> Logomark</p>
  <h2>Primary and small-size redraw</h2>
  <p class="note">Below 32px the 3-unit cut starts closing under antialiasing, so 24
  and 16 use a redraw with the cut opened to 4. Every other edge is identical.
  Crossover is at 32.</p>
  <div class="stack">
    <div class="boards">
      <div class="board light">{m(88)}</div>
      <div class="board dark">{m(88)}</div>
    </div>
    <p class="cap"><span>assets/svg/mark.svg</span><span>32px and above</span></p>
  </div>
  <div class="ladder">{LADDER}</div>
  <p class="cap"><span>orange labels use mark-sm.svg</span><span>below 16px: solid Haar square, no cut</span></p>
</section>

<section>
  <p class="eye"><b>&sect; 2.3</b> Wordmark</p>
  <h2>Archivo Medium, lowercase, &minus;0.03em</h2>
  <p class="note">Shipped as outlines, so there is no font dependency and it cannot
  re-render differently anywhere. Lowercase belongs to the logotype only; in running
  text the name is always written Brume.</p>
  <div class="boards">
    <div class="board light">{wm()}</div>
    <div class="board dark">{wm()}</div>
  </div>
  <p class="cap"><span>assets/svg/wordmark.svg</span><span>SIL OFL 1.1 &middot; free for commercial use</span></p>
</section>

<section>
  <p class="eye"><b>&sect; 2.4</b> Lockups</p>
  <h2>Horizontal and stacked</h2>
  <p class="note">Horizontal sets the mark at ascender height with a 3-module gap.
  Stacked takes the mark to 1.5&times; so it holds its own against the wordmark's
  width. These two are the only approved lockups.</p>
  <div class="stack">
    <div class="boards">
      <div class="board light">{lk("h")}</div>
      <div class="board dark">{lk("h")}</div>
    </div>
    <p class="cap"><span>assets/svg/lockup-h.svg</span><span>header &middot; social card &middot; min 120px wide</span></p>
  </div>
  <div class="stack">
    <div class="boards">
      <div class="board light" style="min-height:190px">{lk("v","lk lkv")}</div>
      <div class="board dark" style="min-height:190px">{lk("v","lk lkv")}</div>
    </div>
    <p class="cap"><span>assets/svg/lockup-v.svg</span><span>README &middot; splash &middot; min 90px wide</span></p>
  </div>
</section>

<section>
  <p class="eye"><b>&sect; 2.5</b> App tile</p>
  <h2>OS icon slots only</h2>
  <p class="note">Radius 21.9% of canvas, matching the platform squircle masks. Glyph
  fills 62.5%. This container does not belong in a README, a header or a footer.</p>
  <div class="boards">
    <div class="board light" style="min-height:210px">
      <svg width="132" height="132" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="{K.INK}"/>
      <g transform="translate(5.3333 5.3333) scale(1.66667)" fill="{K.PAPER}"><path d="{A} {B}"/></g></svg>
    </div>
    <div class="board dark" style="min-height:210px">
      <svg width="132" height="132" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="{K.PAPER}"/>
      <g transform="translate(5.3333 5.3333) scale(1.66667)" fill="{K.INK}"><path d="{A} {B}"/></g></svg>
    </div>
  </div>
  <p class="cap"><span>tile-dark.svg &middot; brume.ico &middot; icon-192/512.png</span><span>min 32px</span></p>
</section>

<section>
  <p class="eye"><b>&sect; 4</b> Colour</p>
  <h2>Four values, one of them rationed</h2>
  <p class="note">Haar is daylight with the information taken out: 18% saturation,
  deliberately not a tech blue. Lamplight appears once per screen at most.</p>
  <div class="swatches">{SWATCHES}</div>
  <table><thead><tr><th>Colour</th><th>On Paper</th><th>On Ink</th><th>Safe for</th></tr></thead><tbody>
    <tr><td>Ink</td><td class="n pass">16.4:1</td><td class="n">&mdash;</td><td>body text on light, AAA</td></tr>
    <tr><td>Haar</td><td class="n pass">6.5:1</td><td class="n warn">2.8:1</td><td>text on light only</td></tr>
    <tr><td>Haar reversed <code>#9DB2C0</code></td><td class="n warn">2.1:1</td><td class="n pass">8.7:1</td><td>text on dark only</td></tr>
    <tr><td>Lamplight</td><td class="n warn">2.0:1</td><td class="n pass">8.2:1</td><td>text on dark; graphic use on light</td></tr>
    <tr><td>Paper</td><td class="n">&mdash;</td><td class="n pass">16.4:1</td><td>body text on dark, AAA</td></tr>
  </tbody></table>
</section>

<section>
  <p class="eye"><b>&sect; 5</b> Typography</p>
  <h2>Archivo everywhere, one variable file</h2>
  <p class="note">The wordmark's own face does the whole system: 100 to 900 weight,
  62 to 125 width, one file, SIL OFL. IBM Plex Mono handles code. That is two
  licences and roughly 60KB subset for the entire identity.</p>
  <div class="spec">
    <div class="row"><span class="lab">display 40/44</span><span class="d1">Nothing to see here</span></div>
    <div class="row"><span class="lab">h1 28/34</span><span class="d2">A browser that forgets you</span></div>
    <div class="row"><span class="lab">h2 20/28</span><span class="d3">Trackers blocked by default</span></div>
    <div class="row"><span class="lab">body 16/26</span><span class="d4">Brume is an old word for mist:
      the kind that softens outlines and makes distance unreadable. The browser is named
      for the effect, not the weather.</span></div>
    <div class="row"><span class="lab">micro 13/20</span><span class="d5">Londev &middot; MIT licensed &middot; Windows 10 and later</span></div>
    <div class="row"><span class="lab">mono</span><span style="font-family:var(--mono);font-size:13px">cargo tauri build --target x86_64-pc-windows-msvc</span></div>
  </div>
</section>

<section>
  <p class="eye"><b>&sect; 6</b> Iconography</p>
  <h2>Lucide, on the same grid</h2>
  <p class="note">The set is Lucide, ISC licensed. 24 grid, stroke 2, round caps and
  joins, currentColor, which is the geometry the chrome already masked against. The
  hand-drawn set this replaced was orthogonal to match the mark, and squaring off
  things that are conventionally round reads as deliberate once and as a limitation
  across forty. State is colour, never an added element.</p>
  <div class="iconset">{ICONSET}</div>
  <p class="cap"><span>assets/icons/ &middot; Lucide, ISC &middot; 24 grid, stroke 2</span>
  <span>cleave.svg is the mark itself, filled, not a UI icon and not Lucide</span></p>
  <div class="boards" style="margin-top:14px">
    <div class="board light">{ic("shield", 44)}</div>
    <div class="board dark" style="color:{K.LAMPLIGHT}">{ic("shield", 44)}</div>
  </div>
  <p class="cap"><span>state is colour, never an added element</span><span>default / active</span></p>
</section>

<section>
  <p class="eye"><b>&sect; 7</b> In context</p>
  <h2>Browser chrome</h2>
  <p class="note">The logo does not live in the toolbar. It appears in the favicon, on
  the new-tab page as a watermark, and in the about dialog. Nowhere else in the product.</p>
  <div class="mock stack">
    <div class="chrome">
      <div class="tabs">
        <div class="tab on">{m(16, True)}<span>New tab</span></div>
        <div class="tab">{m(16, True)}<span>Settings</span></div>
      </div>
      <div class="bar">
        <span style="color:{K.LAMPLIGHT};display:flex">{ic("shield", 18)}</span>
        <div class="url">Search or enter address</div>
      </div>
    </div>
    <div class="page"><span class="wmk">{m(48)}</span><div class="field"></div></div>
  </div>
  <p class="cap"><span>16px favicon &middot; 48px watermark at 42%</span><span>Lamplight used once, on the shield</span></p>

  <h2 style="margin-top:56px">README header</h2>
  <p class="note">Stacked lockup at 180px, one tagline, greyscale badges. No banner,
  no screenshot above the fold, no emoji.</p>
  <div class="mock readme stack">
    <div style="color:{K.INK}">{lk("v","lk lkv")}</div>
    <p class="tag">A lightweight, privacy-focused browser. Built by Londev.</p>
    <div class="badges"><span class="badge">MIT</span><span class="badge">windows</span>
    <span class="badge">tauri</span><span class="badge">v0.1.0</span></div>
  </div>

  <h2 style="margin-top:56px">Icon slots</h2>
  <p class="note">Taskbar, dock, installer. The 16 and 24 entries inside brume.ico use
  the small redraw, not a downscale of the 256.</p>
  <div class="mock dock">
    <svg width="96" height="96" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="{K.INK}" stroke="#232A30"/><g transform="translate(5.3333 5.3333) scale(1.66667)" fill="{K.PAPER}"><path d="{A} {B}"/></g></svg>
    <svg width="64" height="64" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="{K.INK}" stroke="#232A30"/><g transform="translate(5.3333 5.3333) scale(1.66667)" fill="{K.PAPER}"><path d="{A} {B}"/></g></svg>
    <svg width="32" height="32" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="{K.INK}" stroke="#232A30"/><g transform="translate(5.3333 5.3333) scale(1.66667)" fill="{K.PAPER}"><path d="{ASM} {BSM}"/></g></svg>
    <svg width="24" height="24" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="{K.INK}" stroke="#232A30"/><g transform="translate(5.3333 5.3333) scale(1.66667)" fill="{K.PAPER}"><path d="{ASM} {BSM}"/></g></svg>
  </div>
</section>

<section>
  <p class="eye"><b>&sect; 8</b> Usage</p>
  <h2>Clear space is 3 modules</h2>
  <p class="note">Nine mark-units on all four sides, which is three times the width of
  the cut and works out to about 37% of the mark's height. It scales correctly at any
  size because the module scales with the mark.</p>
  <div class="rules">
    <div class="do"><h3>Do</h3><ul>
      <li>Use <code>currentColor</code> and set colour in CSS. One file, every theme.</li>
      <li>Redraw below 32px rather than scaling down.</li>
      <li>Keep every hex code in <code>tokens.css</code> and nowhere else.</li>
      <li>Let the mark sit alone. It is strongest with nothing near it.</li>
    </ul></div>
    <div class="dont"><h3>Don't</h3><ul>
      <li>Close the cut, widen it, or change its step. That is the entire mark.</li>
      <li>Rotate. The two pieces are congruent under 180&deg;, so a rotated mark is
      indistinguishable from the original and reads as a mistake.</li>
      <li>Stretch or skew. The cut only stays one module wide under uniform scale.</li>
      <li>Add shadows, glows, gradients or frosted-glass backing.</li>
      <li>Fill the two pieces in different colours. It is one mark, not two shapes.</li>
      <li>Put the app tile in a README, a header or a footer.</li>
    </ul></div>
  </div>
</section>

<footer><span style="color:var(--fg)">{m(20)}</span><p>Brume brand kit v2.0 &middot; Londev</p></footer>
</div><script>
(function(){{
  var r=document.documentElement,b=document.getElementById('tg');
  b.addEventListener('click',function(){{
    var n=r.getAttribute('data-theme')==='light'?'dark':'light';
    r.setAttribute('data-theme',n);b.textContent=n==='light'?'Dark':'Light';
  }});
  var rig=document.getElementById('rig'),txt=document.getElementById('rigtext');
  var copy={{
    assembled:"The mark as it ships. Reads as one solid form with a hairline running "+
      "through it. At a glance it is a square; the second look is the cut.",
    apart:"The same two pieces pulled apart. The cut steps down, across, then down "+
      "again, so neither piece is a rectangle and neither is a simple L.",
    proof:"Piece B rotated 180 degrees about the centre. It lands exactly on piece A: "+
      "the two halves are the same shape. That is the idea the mark is carrying."
  }};
  rig.querySelectorAll('.steps button').forEach(function(btn){{
    btn.addEventListener('click',function(){{
      var s=btn.dataset.s;
      rig.setAttribute('data-state',s);
      txt.textContent=copy[s];
      rig.querySelectorAll('.steps button').forEach(function(o){{
        o.setAttribute('aria-pressed',String(o===btn));
      }});
    }});
  }});
}})();
</script></body></html>
'''

open(f"{ROOT}/preview.html", "w").write(HTML)
print(f"preview.html: {len(HTML):,} bytes")
