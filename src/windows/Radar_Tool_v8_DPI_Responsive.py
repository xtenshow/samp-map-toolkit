"""
GTA SA Radar Tool v8.0 — Premium Edition
Designed & built by Tensho Records
"""

import os
import ctypes
import struct
import webbrowser
import colorsys
import traceback
from PIL import Image
import tkinter as tk
from tkinter import filedialog, messagebox, colorchooser


# ============================================================
#  CONSTANTS
# ============================================================
TELEGRAM_URL = "https://t.me/TenshoRecords"
DEFAULT_TILE_SIZE = 256
GRID_SIZE = 12
TOTAL_TILES = GRID_SIZE * GRID_SIZE

# ── Premium palette ──────────────────────────────────────
BG_DEEP       = "#07080d"
BG_SIDEBAR    = "#0b0c12"
BG_MAIN       = "#0e0f16"
BG_CARD       = "#171922"
BG_CARD_HOV   = "#1e202b"
BG_INPUT      = "#1c1e2a"
BORDER        = "#22252f"
BORDER_STRONG = "#2d3140"
FG            = "#e8eaf5"
FG_DIM        = "#9498b0"
FG_MUTED      = "#5f6380"
PRIMARY       = "#8b5cf6"
PRIMARY_HOVER = "#a78bfa"
PRIMARY_DIM   = "#3d2a70"
GREEN         = "#10b981"
GREEN_HOVER   = "#34d399"
CYAN          = "#22d3ee"
PINK          = "#ec4899"
RED           = "#ef4444"
YELLOW        = "#f59e0b"
TELE          = "#229ed9"
TELE_HOVER    = "#4fb8e8"
WHITE         = "#ffffff"

# ── RenderWare / TXD constants ──────────────────────────
RW_VERSION           = 0x1803FFFF
SECTION_STRUCT       = 0x01
SECTION_EXTENSION    = 0x03
SECTION_TEX_NATIVE   = 0x15
SECTION_TEX_DICT     = 0x16
PLATFORM_D3D9        = 9
RASTER_FORMAT        = 0x0500
D3DFMT_A8R8G8B8      = 21
FILTER_FLAGS         = 0x1106
DEPTH                = 32
MIPMAP_COUNT         = 1
TEX_TYPE             = 4
D3D9_FLAGS           = 0x01


# ============================================================
#  UTILITIES
# ============================================================
def hex_to_rgb(h):
    return int(h[1:3], 16), int(h[3:5], 16), int(h[5:7], 16)


def lerp_color(c1, c2, t):
    r1, g1, b1 = hex_to_rgb(c1)
    r2, g2, b2 = hex_to_rgb(c2)
    return "#{:02x}{:02x}{:02x}".format(
        int(r1 + (r2 - r1) * t),
        int(g1 + (g2 - g1) * t),
        int(b1 + (b2 - b1) * t),
    )


def draw_round_rect(canvas, x1, y1, x2, y2, r, fill, outline="", width=0, tags=""):
    if r < 1:
        r = 1
    if 2 * r > (x2 - x1):
        r = max(1, (x2 - x1) // 2)
    if 2 * r > (y2 - y1):
        r = max(1, (y2 - y1) // 2)

    if fill:
        canvas.create_arc(x1, y1, x1+2*r, y1+2*r, start=90, extent=90,
                          fill=fill, outline="", tags=tags)
        canvas.create_arc(x2-2*r, y1, x2, y1+2*r, start=0, extent=90,
                          fill=fill, outline="", tags=tags)
        canvas.create_arc(x1, y2-2*r, x1+2*r, y2, start=180, extent=90,
                          fill=fill, outline="", tags=tags)
        canvas.create_arc(x2-2*r, y2-2*r, x2, y2, start=270, extent=90,
                          fill=fill, outline="", tags=tags)
        canvas.create_rectangle(x1+r, y1, x2-r, y2, fill=fill, outline="", tags=tags)
        canvas.create_rectangle(x1, y1+r, x2, y2-r, fill=fill, outline="", tags=tags)

    if outline and width > 0:
        canvas.create_arc(x1, y1, x1+2*r, y1+2*r, start=90, extent=90,
                          style="arc", outline=outline, width=width, tags=tags)
        canvas.create_arc(x2-2*r, y1, x2, y1+2*r, start=0, extent=90,
                          style="arc", outline=outline, width=width, tags=tags)
        canvas.create_arc(x1, y2-2*r, x1+2*r, y2, start=180, extent=90,
                          style="arc", outline=outline, width=width, tags=tags)
        canvas.create_arc(x2-2*r, y2-2*r, x2, y2, start=270, extent=90,
                          style="arc", outline=outline, width=width, tags=tags)
        canvas.create_line(x1+r, y1, x2-r, y1, fill=outline, width=width, tags=tags)
        canvas.create_line(x1+r, y2, x2-r, y2, fill=outline, width=width, tags=tags)
        canvas.create_line(x1, y1+r, x1, y2-r, fill=outline, width=width, tags=tags)
        canvas.create_line(x2, y1+r, x2, y2-r, fill=outline, width=width, tags=tags)


def draw_empty_state(canvas, text, w=380, h=200):
    canvas.delete("all")
    canvas.create_rectangle(8, 8, w-8, h-8, outline=FG_MUTED, dash=(3, 5), width=1)
    canvas.create_text(w // 2, h // 2 - 14, text="📂", fill=FG_MUTED,
                       font=("Segoe UI Emoji", 22))
    canvas.create_text(w // 2, h // 2 + 14, text=text, fill=FG_MUTED,
                       font=("Segoe UI", 9))


# ============================================================
#  TXD BUILDERS
# ============================================================
def _rw_section(section_type, data):
    return struct.pack("<III", section_type, len(data), RW_VERSION) + data


def _make_txd(texture_name, image):
    img = image.convert("RGBA")
    r, g, b, a = img.split()
    pixel_data = Image.merge("RGBA", (b, g, r, a)).tobytes()
    w, h = img.size
    name_bytes = texture_name.encode("ascii")[:31].ljust(32, b"\x00")
    alpha_bytes = b"\x00" * 32

    native = (
        struct.pack("<II", PLATFORM_D3D9, FILTER_FLAGS)
        + name_bytes + alpha_bytes
        + struct.pack("<IIHHBBBBI",
                      RASTER_FORMAT, D3DFMT_A8R8G8B8, w, h,
                      DEPTH, MIPMAP_COUNT, TEX_TYPE, D3D9_FLAGS, len(pixel_data))
        + pixel_data
    )

    tex_native = _rw_section(
        SECTION_TEX_NATIVE,
        _rw_section(SECTION_STRUCT, native) + _rw_section(SECTION_EXTENSION, b"")
    )
    tex_dict = _rw_section(
        SECTION_TEX_DICT,
        _rw_section(SECTION_STRUCT, struct.pack("<HH", 1, 2))
        + tex_native + _rw_section(SECTION_EXTENSION, b"")
    )
    return tex_dict


# ============================================================
#  WIDGETS
# ============================================================
class PillButton(tk.Canvas):
    def __init__(self, parent, text, command, bg_color, fg_color,
                 hover_color=None, width=180, height=42, radius=None,
                 font=("Segoe UI", 10, "bold")):
        bg = parent.cget("bg") if hasattr(parent, "cget") else BG_MAIN
        super().__init__(parent, width=width, height=height, bg=bg,
                         highlightthickness=0, bd=0, cursor="hand2")
        self._text = text
        self._command = command
        self._bg_color = bg_color
        self._fg_color = fg_color
        self._hover_color = hover_color or bg_color
        
        # متغیرهای عرض و ارتفاع (تغییر نام یافته به _cw و _ch)
        self._cw = width
        self._ch = height
        
        self._radius = radius if radius is not None else height // 2
        self._font = font
        self._current = bg_color
        self._anim_job = None

        self._draw()
        self.bind("<Enter>", lambda e: self._animate_to(self._hover_color))
        self.bind("<Leave>", lambda e: self._animate_to(self._bg_color))
        self.bind("<Button-1>", self._on_click)

    def _on_click(self, e):
        if self._command:
            self._command()

    def _draw(self):
        self.delete("all")
        draw_round_rect(self, 0, 0, self._cw - 1, self._ch - 1,
                        self._radius, self._current, "", 0)
        self.create_text(self._cw // 2, self._ch // 2, text=self._text,
                         fill=self._fg_color, font=self._font)

    def _animate_to(self, target, steps=10, delay=14):
        if self._anim_job is not None:
            try:
                self.after_cancel(self._anim_job)
            except Exception:
                pass
            self._anim_job = None

        start = self._current

        def step(i):
            if i > steps:
                self._anim_job = None
                return
            t = i / steps
            t_eased = 1 - (1 - t) ** 2
            self._current = lerp_color(start, target, t_eased)
            try:
                self._draw()
                self._anim_job = self.after(delay, lambda: step(i + 1))
            except tk.TclError:
                self._anim_job = None

        step(0)


class PillToggle(tk.Canvas):
    def __init__(self, parent, variable, width=52, height=28):
        bg = parent.cget("bg")
        super().__init__(parent, width=width, height=height, bg=bg,
                         highlightthickness=0, cursor="hand2")
        self._var = variable
        
        # متغیرهای عرض و ارتفاع (تغییر نام یافته)
        self._cw = width
        self._ch = height
        
        self._pos = 1.0 if variable.get() else 0.0
        self._anim_job = None
        self._draw()
        self.bind("<Button-1>", lambda e: self._var.set(not self._var.get()))
        self._var.trace_add("write", lambda *a: self._animate(self._var.get()))

    def _draw(self):
        self.delete("all")
        w, h = self._cw, self._ch
        r = h // 2
        track = lerp_color(BG_INPUT, PRIMARY, self._pos)
        draw_round_rect(self, 0, 0, w, h, r, track, "", 0)
        kr = r - 4
        cx = r + (w - 2 * r) * self._pos
        self.create_oval(cx - kr, r - kr, cx + kr, r + kr, fill=WHITE, outline="")

    def _animate(self, on):
        if self._anim_job is not None:
            try:
                self.after_cancel(self._anim_job)
            except Exception:
                pass
            self._anim_job = None

        target = 1.0 if on else 0.0
        start = self._pos

        def step(i, total=10):
            if i > total:
                self._anim_job = None
                return
            t = i / total
            eased = 1 - (1 - t) ** 3
            self._pos = start + (target - start) * eased
            try:
                self._draw()
                self._anim_job = self.after(16, lambda: step(i + 1))
            except tk.TclError:
                self._anim_job = None

        step(0)


class RoundedCard(tk.Frame):
    """Frame with a canvas-drawn rounded background."""
    def __init__(self, parent, fill=BG_CARD, border=None, radius=14, padding=16,
                 bg=None, **kw):
        if bg is None:
            bg = parent.cget("bg") if hasattr(parent, "cget") else BG_MAIN
        super().__init__(parent, bg=bg, **kw)
        self._fill = fill
        self._border = border
        self._radius = radius
        self._padding = padding

        self._canvas = tk.Canvas(self, bg=bg, highlightthickness=0, bd=0)
        self._canvas.place(x=0, y=0, relwidth=1, relheight=1)

        self.body = tk.Frame(self, bg=fill)
        self.body.pack(fill="both", expand=True, padx=padding, pady=padding)

        self.bind("<Configure>", self._on_resize)

    def _on_resize(self, e):
        w, h = e.width, e.height
        if w < 4 or h < 4:
            return
        self._canvas.delete("bg")
        draw_round_rect(self._canvas, 0, 0, w, h, self._radius,
                        self._fill, self._border or "",
                        1 if self._border else 0, "bg")
        self._canvas.tag_lower("bg")


class ColorSwatch(tk.Canvas):
    def __init__(self, parent, color, size=48, on_click=None):
        bg = parent.cget("bg")
        super().__init__(parent, width=size, height=size, bg=bg,
                         highlightthickness=0,
                         cursor="hand2" if on_click else "arrow")
        self._size = size
        self._color = color
        self._on_click = on_click
        self._hover = False
        self._draw()
        if on_click:
            self.bind("<Button-1>", lambda e: on_click())
            self.bind("<Enter>", lambda e: self._set_hover(True))
            self.bind("<Leave>", lambda e: self._set_hover(False))

    def _set_hover(self, h):
        self._hover = h
        self._draw()

    def _draw(self):
        self.delete("all")
        border = PRIMARY if self._hover else BORDER_STRONG
        w = 2 if self._hover else 1
        draw_round_rect(self, 0, 0, self._size - 1, self._size - 1,
                        10, self._color, border, w)

    def set_color(self, color):
        self._color = color
        self._draw()


class CustomSlider(tk.Canvas):
    def __init__(self, parent, variable, from_=0, to=100,
                 width=400, height=32, on_change=None):
        bg = parent.cget("bg")
        super().__init__(parent, width=width, height=height, bg=bg,
                         highlightthickness=0, cursor="hand2")
        self._var = variable
        self._min = from_
        self._max = to
        
        # متغیرهای عرض و ارتفاع (تغییر نام یافته)
        self._cw = width
        self._ch = height
        
        self._on_change = on_change
        self._draw()
        self.bind("<Button-1>", self._on_press)
        self.bind("<B1-Motion>", self._on_press)
        self.bind("<Configure>", self._on_configure)
        self._var.trace_add("write", lambda *a: self._update())

    def _on_configure(self, e):
        self._cw = max(20, e.width)
        self._ch = max(16, e.height)
        self._draw()

    def _draw(self):
        self.delete("all")
        w, h = self._cw, self._ch
        th = 6
        ty = h // 2
        tr = 9

        draw_round_rect(self, 4, ty - th // 2, w - 4, ty + th // 2,
                        th // 2, BG_INPUT, "", 0)

        val = self._var.get()
        pct = (val - self._min) / max(1, (self._max - self._min))
        tx = 4 + (w - 8) * pct
        if tx > 4:
            draw_round_rect(self, 4, ty - th // 2, int(tx), ty + th // 2,
                            th // 2, PRIMARY, "", 0)

        self.create_oval(tx - tr, ty - tr, tx + tr, ty + tr,
                         fill=WHITE, outline=PRIMARY, width=2)

    def _update(self):
        try:
            self._draw()
        except tk.TclError:
            return
        if self._on_change:
            self._on_change(self._var.get())

    def _on_press(self, e):
        pct = max(0.0, min(1.0, (e.x - 4) / max(1, (self._cw - 8))))
        val = self._min + (self._max - self._min) * pct
        self._var.set(int(round(val)))


class StatusBar(tk.Frame):
    def __init__(self, parent):
        super().__init__(parent, bg=BG_SIDEBAR, height=34)
        self.pack_propagate(False)

        left = tk.Frame(self, bg=BG_SIDEBAR)
        left.pack(side="left", fill="y", padx=22)

        self._dot_canvas = tk.Canvas(left, width=12, height=12,
                                      bg=BG_SIDEBAR, highlightthickness=0)
        self._dot_canvas.pack(side="left", pady=11)
        self._dot = self._dot_canvas.create_oval(3, 3, 9, 9, fill=GREEN, outline="")

        self._label = tk.Label(left, text="Ready",
                                bg=BG_SIDEBAR, fg=FG_DIM,
                                font=("Segoe UI", 9))
        self._label.pack(side="left", padx=(8, 0))

        right = tk.Frame(self, bg=BG_SIDEBAR)
        right.pack(side="right", fill="y", padx=22)
        tk.Label(right, text="v8.0  ·  Premium Edition",
                 bg=BG_SIDEBAR, fg=FG_MUTED,
                 font=("Segoe UI", 8)).pack(side="right", pady=9)

    def set_status(self, text, color=GREEN):
        self._label.config(text=text)
        self._dot_canvas.itemconfig(self._dot, fill=color)


class ScrollableArea(tk.Frame):
    def __init__(self, parent, bg):
        super().__init__(parent, bg=bg)

        self._scrollbar_visible = False

        self._canvas = tk.Canvas(self, bg=bg, highlightthickness=0, bd=0)
        self._scrollbar = tk.Scrollbar(
            self, orient="vertical", command=self._canvas.yview,
            bg=BG_CARD, troughcolor=BG_MAIN, activebackground=PRIMARY,
            width=8, bd=0, relief="flat",
        )
        self._canvas.configure(yscrollcommand=self._scrollbar.set)

        self.body = tk.Frame(self._canvas, bg=bg)
        self._body_id = self._canvas.create_window(0, 0, window=self.body, anchor="nw")

        self._canvas.pack(side="left", fill="both", expand=True)
        self._canvas.bind("<Configure>", self._on_canvas_cfg)
        self.body.bind("<Configure>", self._on_body_cfg)

    def _on_canvas_cfg(self, e):
        self._canvas.itemconfig(self._body_id, width=e.width)
        self._check_scrollbar()

    def _on_body_cfg(self, e):
        self._canvas.configure(scrollregion=self._canvas.bbox("all"))
        self._check_scrollbar()

    def _check_scrollbar(self):
        need = self.body.winfo_reqheight() > self._canvas.winfo_height() + 2
        if need and not self._scrollbar_visible:
            self._scrollbar.pack(side="right", fill="y")
            self._scrollbar_visible = True
        elif not need and self._scrollbar_visible:
            self._scrollbar.pack_forget()
            self._scrollbar_visible = False

    def scroll_wheel(self, delta):
        if self._scrollbar_visible:
            self._canvas.yview_scroll(int(-1 * (delta / 120)), "units")


# ============================================================
#  MAIN APP
# ============================================================
class RadarTool:
    def __init__(self, root):
        self.root = root
        self.root.title("Radar Tool — GTA SA  ·  Tensho Records")
        self.root.configure(bg=BG_DEEP)

        # Adapt the initial window to the current screen instead of assuming
        # that every user has enough vertical space for a fixed 1240x880 window.
        screen_w = max(640, self.root.winfo_screenwidth())
        screen_h = max(480, self.root.winfo_screenheight())
        min_w = min(1060, max(760, screen_w - 48))
        min_h = min(780, max(560, screen_h - 96))
        self.root.minsize(min_w, min_h)

        if os.name == "nt":
            # DPI awareness is enabled before Tk is created (see __main__).
            # Start maximized on Windows without forcing exclusive fullscreen.
            try:
                self.root.state("zoomed")
            except tk.TclError:
                width = min(1440, max(900, screen_w - 64))
                height = min(940, max(640, screen_h - 96))
                x = max(0, (screen_w - width) // 2)
                y = max(0, (screen_h - height) // 2)
                self.root.geometry(f"{width}x{height}+{x}+{y}")
        else:
            width = min(1440, max(900, screen_w - 64))
            height = min(940, max(640, screen_h - 96))
            x = max(0, (screen_w - width) // 2)
            y = max(0, (screen_h - height) // 2)
            self.root.geometry(f"{width}x{height}+{x}+{y}")

        # Folder state
        self.input_folder = ""
        self.output_folder = ""
        self.txd_input_folder = ""
        self.txd_output_folder = ""

        # Color page state
        self._cr_image_path = ""
        self._cr_pil_image = None
        self._cr_preview_src = None
        self._cr_preview_job = None
        self._cr_thumb_resize_job = None
        self._cr_live_resize_job = None
        self._cr_target_rgb = (128, 128, 128)
        self._cr_replace_rgb = (255, 0, 0)
        self._cr_tk_thumb = None
        self._cr_live_tk = None
        self._cr_thumb_size = (0, 0)
        self._cr_thumb_offset = (0, 0)

        # Nav state
        self._current_page = None
        self._nav_items = {}
        self._pages = {}

        self._build_ui()

    # ── UI Layout ────────────────────────────────────────
    def _build_ui(self):
        content = tk.Frame(self.root, bg=BG_DEEP)
        content.pack(fill="both", expand=True)

        self._build_sidebar(content)
        self._build_main(content)
        self._build_statusbar()

        # Cross-platform scroll wheel
        self.root.bind_all("<MouseWheel>", self._on_wheel)
        self.root.bind_all("<Button-4>", lambda e: self._scroll.scroll_wheel(120))
        self.root.bind_all("<Button-5>", lambda e: self._scroll.scroll_wheel(-120))

        self._switch_page("merge_split")

    def _build_sidebar(self, parent):
        s = tk.Frame(parent, bg=BG_SIDEBAR, width=240)
        s.pack(side="left", fill="y")
        s.pack_propagate(False)

        # Logo
        logo = tk.Frame(s, bg=BG_SIDEBAR)
        logo.pack(fill="x", padx=22, pady=(26, 0))

        badge = tk.Canvas(logo, width=46, height=46, bg=BG_SIDEBAR,
                           highlightthickness=0)
        badge.pack(side="left")
        draw_round_rect(badge, 0, 0, 46, 46, 13, PRIMARY, "", 0)
        badge.create_text(23, 23, text="🛰️", font=("Segoe UI Emoji", 22))

        title = tk.Frame(logo, bg=BG_SIDEBAR)
        title.pack(side="left", padx=(14, 0))
        tk.Label(title, text="RADAR", bg=BG_SIDEBAR, fg=FG,
                 font=("Segoe UI", 14, "bold")).pack(anchor="w")
        tk.Label(title, text="TOOL  ·  v8", bg=BG_SIDEBAR, fg=PRIMARY,
                 font=("Segoe UI", 9, "bold")).pack(anchor="w")

        # Divider
        tk.Frame(s, bg=BORDER, height=1).pack(fill="x", padx=22, pady=(26, 10))

        # Nav header
        tk.Label(s, text="TOOLS", bg=BG_SIDEBAR, fg=FG_MUTED,
                 font=("Segoe UI", 8, "bold")).pack(anchor="w", padx=26, pady=(8, 8))

        # Nav items
        nav = tk.Frame(s, bg=BG_SIDEBAR)
        nav.pack(fill="x", padx=10)

        self._make_nav_item(nav, "merge_split", "🗺️", "Merge / Split")
        self._make_nav_item(nav, "txd", "📦", "PNG → TXD")
        self._make_nav_item(nav, "color", "🎨", "Color Map")

        # Spacer
        tk.Frame(s, bg=BG_SIDEBAR).pack(fill="both", expand=True)

        # Footer
        tk.Frame(s, bg=BORDER, height=1).pack(fill="x", padx=22, pady=(10, 14))

        foot = tk.Frame(s, bg=BG_SIDEBAR)
        foot.pack(fill="x", padx=18, pady=(0, 20))

        PillButton(foot, "✈   @TenshoRecords", self.open_telegram,
                   bg_color=TELE, fg_color=WHITE, hover_color=TELE_HOVER,
                   width=204, height=38, radius=10,
                   font=("Segoe UI", 9, "bold")).pack()

        tk.Label(foot, text="Made with ♥ by Tensho Records",
                 bg=BG_SIDEBAR, fg=FG_MUTED,
                 font=("Segoe UI", 8)).pack(pady=(10, 0))

    def _make_nav_item(self, parent, page_id, icon, text):
        outer = tk.Frame(parent, bg=BG_SIDEBAR, cursor="hand2")
        outer.pack(fill="x", pady=2)

        indicator = tk.Frame(outer, bg=BG_SIDEBAR, width=3)
        indicator.pack(side="left", fill="y")
        indicator.pack_propagate(False)

        inner = tk.Frame(outer, bg=BG_SIDEBAR)
        inner.pack(side="left", fill="x", expand=True, padx=(0, 0), pady=2)

        content = tk.Frame(inner, bg=BG_SIDEBAR)
        content.pack(fill="x", padx=10, pady=10)

        ic_lbl = tk.Label(content, text=icon, bg=BG_SIDEBAR, fg=FG_DIM,
                           font=("Segoe UI Emoji", 13))
        ic_lbl.pack(side="left")

        tx_lbl = tk.Label(content, text=text, bg=BG_SIDEBAR, fg=FG_DIM,
                           font=("Segoe UI", 10))
        tx_lbl.pack(side="left", padx=(10, 0))

        def set_state(hover=False, active=False):
            if active:
                inner.config(bg=BG_CARD)
                indicator.config(bg=PRIMARY)
                content.config(bg=BG_CARD)
                ic_lbl.config(bg=BG_CARD, fg=PRIMARY)
                tx_lbl.config(bg=BG_CARD, fg=FG, font=("Segoe UI", 10, "bold"))
            elif hover:
                content.config(bg=BG_CARD_HOV)
                ic_lbl.config(bg=BG_CARD_HOV, fg=FG)
                tx_lbl.config(bg=BG_CARD_HOV, fg=FG)
            else:
                inner.config(bg=BG_SIDEBAR)
                indicator.config(bg=BG_SIDEBAR)
                content.config(bg=BG_SIDEBAR)
                ic_lbl.config(bg=BG_SIDEBAR, fg=FG_DIM)
                tx_lbl.config(bg=BG_SIDEBAR, fg=FG_DIM,
                              font=("Segoe UI", 10))

        on_enter = lambda e: (set_state(hover=True)
                              if self._current_page != page_id else None)
        on_leave = lambda e: (set_state()
                              if self._current_page != page_id else None)
        on_click = lambda e: self._switch_page(page_id)

        for w in (outer, inner, content, ic_lbl, tx_lbl):
            w.bind("<Enter>", on_enter)
            w.bind("<Leave>", on_leave)
            w.bind("<Button-1>", on_click)

        self._nav_items[page_id] = set_state

    def _build_main(self, parent):
        main = tk.Frame(parent, bg=BG_MAIN)
        main.pack(side="left", fill="both", expand=True)

        self._header_frame = tk.Frame(main, bg=BG_MAIN)
        self._header_frame.pack(fill="x", padx=32, pady=(28, 0))

        self._scroll = ScrollableArea(main, BG_MAIN)
        self._scroll.pack(fill="both", expand=True, pady=(14, 0))

        self._page_container = self._scroll.body

        self._build_page_merge_split()
        self._build_page_txd()
        self._build_page_color()

    def _build_statusbar(self):
        self._status_bar = StatusBar(self.root)
        self._status_bar.pack(fill="x", side="bottom")

    def _set_status(self, text, color=GREEN):
        self._status_bar.set_status(text, color)

    def _on_wheel(self, e):
        self._scroll.scroll_wheel(e.delta)

    def _switch_page(self, page_id):
        if self._current_page == page_id:
            return
        for f in self._pages.values():
            f.pack_forget()
        self._current_page = page_id
        self._pages[page_id].pack(fill="both", expand=True,
                                   padx=32, pady=(0, 32))
        for pid, fn in self._nav_items.items():
            fn(active=(pid == page_id))

        headers = {
            "merge_split": ("01", "🗺️", "Merge / Split",
                            "Combine radar tiles into a full map, or split a map into 144 tiles"),
            "txd": ("02", "📦", "PNG → TXD",
                    "Convert radar PNG files to RenderWare TXD format for GTA SA"),
            "color": ("03", "🎨", "Color Map",
                      "Live recolor — changes update instantly as you adjust"),
        }
        self._build_page_header_content(*headers[page_id])

        if page_id == "color" and self._cr_pil_image is not None:
            self.root.after(60, self._cr_show_thumbnail)
            self.root.after(100, self._cr_schedule_live_preview)

    def _build_page_header_content(self, number, icon, title, subtitle):
        for w in self._header_frame.winfo_children():
            w.destroy()

        row = tk.Frame(self._header_frame, bg=BG_MAIN)
        row.pack(fill="x", anchor="w")

        badge = tk.Canvas(row, width=44, height=30, bg=BG_MAIN,
                           highlightthickness=0)
        badge.pack(side="left")
        draw_round_rect(badge, 0, 0, 44, 30, 15, PRIMARY_DIM, "", 0)
        badge.create_text(22, 15, text=number, fill=PRIMARY_HOVER,
                           font=("Segoe UI", 11, "bold"))

        tk.Label(row, text=f"{icon}   {title}", bg=BG_MAIN, fg=FG,
                 font=("Segoe UI", 22, "bold")).pack(side="left", padx=(14, 0))

        tk.Label(self._header_frame, text=subtitle, bg=BG_MAIN, fg=FG_DIM,
                 font=("Segoe UI", 10)).pack(anchor="w", pady=(4, 0),
                                              padx=(58, 0))

    # ── Reusable widgets ──────────────────────────────────
    def _folder_row(self, parent, icon, title, hint, command):
        row = tk.Frame(parent, bg=parent.cget("bg"))
        row.pack(fill="x")

        tk.Label(row, text=icon, bg=parent.cget("bg"), fg=PRIMARY,
                 font=("Segoe UI Emoji", 14)).pack(side="left", padx=(0, 12))

        info = tk.Frame(row, bg=parent.cget("bg"))
        info.pack(side="left", fill="x", expand=True)

        tk.Label(info, text=title, bg=parent.cget("bg"), fg=FG,
                 font=("Segoe UI", 10, "bold"), anchor="w").pack(fill="x")

        path_label = tk.Label(info, text=hint, bg=parent.cget("bg"), fg=FG_MUTED,
                              font=("Consolas", 9), anchor="w")
        path_label.pack(fill="x", pady=(2, 0))

        PillButton(row, "📂  Browse", command,
                   bg_color=BG_INPUT, fg_color=FG, hover_color=BORDER_STRONG,
                   width=110, height=34, radius=8,
                   font=("Segoe UI", 9, "bold")).pack(side="right", padx=(12, 0))

        return path_label

    # ══════════════════════════════════════════════════════
    #  PAGE: MERGE / SPLIT
    # ══════════════════════════════════════════════════════
    def _build_page_merge_split(self):
        page = tk.Frame(self._page_container, bg=BG_MAIN)
        self._pages["merge_split"] = page

        # Folders
        folders = RoundedCard(page, fill=BG_CARD, radius=14, padding=22)
        folders.pack(fill="x", pady=(0, 12))

        self.input_label = self._folder_row(
            folders.body, "📥", "Input Folder",
            "Select the folder containing radar*.png tiles",
            self.select_input)

        tk.Frame(folders.body, bg=BORDER, height=1).pack(fill="x", pady=14)

        self.output_label = self._folder_row(
            folders.body, "📤", "Output Folder",
            "Where the merged map.png will be saved",
            self.select_output)

        # Settings
        settings = RoundedCard(page, fill=BG_CARD, radius=14, padding=22)
        settings.pack(fill="x", pady=(0, 12))

        hdr = tk.Frame(settings.body, bg=BG_CARD)
        hdr.pack(fill="x")
        tk.Label(hdr, text="📐  TILE SIZE", bg=BG_CARD, fg=FG_DIM,
                 font=("Segoe UI", 9, "bold")).pack(side="left")

        row = tk.Frame(settings.body, bg=BG_CARD)
        row.pack(anchor="w", pady=(12, 0))

        self.tile_w_var = tk.StringVar(value=str(DEFAULT_TILE_SIZE))
        self.tile_h_var = tk.StringVar(value=str(DEFAULT_TILE_SIZE))

        for var in (self.tile_w_var, self.tile_h_var):
            tk.Entry(row, textvariable=var, bg=BG_INPUT, fg=FG,
                     insertbackground=FG, relief="flat", bd=0,
                     font=("Consolas", 12), width=6, justify="center",
                     highlightthickness=1,
                     highlightbackground=BORDER_STRONG,
                     highlightcolor=PRIMARY).pack(side="left", ipady=7)

            if var is self.tile_w_var:
                tk.Label(row, text="×", bg=BG_CARD, fg=FG_DIM,
                         font=("Consolas", 12)).pack(side="left", padx=10)

        tk.Label(row, text=" px", bg=BG_CARD, fg=FG_DIM,
                 font=("Segoe UI", 10)).pack(side="left", padx=(8, 0))

        # Flip toggle
        flip = tk.Frame(settings.body, bg=BG_CARD)
        flip.pack(fill="x", pady=(18, 0))

        self.flip_vertical = tk.BooleanVar(value=False)
        PillToggle(flip, self.flip_vertical, width=48, height=26).pack(side="left")
        tk.Label(flip, text="  Flip Vertical", bg=BG_CARD, fg=FG,
                 font=("Segoe UI", 10)).pack(side="left")
        tk.Label(flip, text="(invert row order)", bg=BG_CARD, fg=FG_MUTED,
                 font=("Segoe UI", 9)).pack(side="left", padx=(8, 0))

        # Actions
        btn_row = tk.Frame(page, bg=BG_MAIN)
        btn_row.pack(fill="x", pady=(0, 12))

        PillButton(btn_row, "🧩   Merge Tiles", self.merge_tiles,
                   bg_color=GREEN, fg_color=BG_DEEP, hover_color=GREEN_HOVER,
                   width=210, height=48, radius=10,
                   font=("Segoe UI", 10, "bold")).pack(side="left", padx=(0, 10))

        PillButton(btn_row, "✂   Split Map", self.split_map,
                   bg_color=PRIMARY, fg_color=WHITE, hover_color=PRIMARY_HOVER,
                   width=210, height=48, radius=10,
                   font=("Segoe UI", 10, "bold")).pack(side="left")

        # Tip
        tip = RoundedCard(page, fill=BG_CARD, radius=12, padding=18)
        tip.pack(fill="x", pady=(4, 0))
        tk.Label(tip.body, text="💡  TIP", bg=BG_CARD, fg=PRIMARY,
                 font=("Segoe UI", 9, "bold")).pack(anchor="w")
        tk.Label(tip.body,
                 text="Tiles must be named radar00.png through radar143.png.\n"
                      "Tile 0 is the top-left of the map. Use Flip Vertical\n"
                      "if your tiles are ordered bottom-up.",
                 bg=BG_CARD, fg=FG_DIM, font=("Segoe UI", 9),                 justify="left").pack(anchor="w", pady=(6, 0))

    # ══════════════════════════════════════════════════════
    #  PAGE: PNG → TXD
    # ══════════════════════════════════════════════════════
    def _build_page_txd(self):
        page = tk.Frame(self._page_container, bg=BG_MAIN)
        self._pages["txd"] = page

        folders = RoundedCard(page, fill=BG_CARD, radius=14, padding=22)
        folders.pack(fill="x", pady=(0, 12))

        self.txd_input_label = self._folder_row(
            folders.body, "🖼", "Input Folder (PNG)",
            "Folder containing radar*.png files",
            self.select_txd_input)

        tk.Frame(folders.body, bg=BORDER, height=1).pack(fill="x", pady=14)

        self.txd_output_label = self._folder_row(
            folders.body, "💾", "Output Folder (TXD)",
            "Where generated .txd files will be saved",
            self.select_txd_output)

        # Info
        info = RoundedCard(page, fill=BG_CARD, radius=12, padding=18)
        info.pack(fill="x", pady=(0, 12))
        tk.Label(info.body, text="ℹ️  HOW IT WORKS", bg=BG_CARD, fg=PRIMARY,
                 font=("Segoe UI", 9, "bold")).pack(anchor="w")
        tk.Label(info.body,
                 text="All files matching radar*.png in the input folder are converted\n"
                      "to individual .txd files with matching names.\n"
                      "Output is compatible with GTA SA (Direct3D 9 native textures).",
                 bg=BG_CARD, fg=FG_DIM, font=("Segoe UI", 9),
                 justify="left").pack(anchor="w", pady=(6, 0))

        # Action
        btn_row = tk.Frame(page, bg=BG_MAIN)
        btn_row.pack(fill="x")
        PillButton(btn_row, "📦   Convert to TXD", self.convert_to_txd,
                   bg_color=PRIMARY, fg_color=WHITE, hover_color=PRIMARY_HOVER,
                   width=230, height=48, radius=10,
                   font=("Segoe UI", 10, "bold")).pack(side="left")

    # ══════════════════════════════════════════════════════
    #  PAGE: COLOR MAP
    # ══════════════════════════════════════════════════════
    def _build_page_color(self):
        page = tk.Frame(self._page_container, bg=BG_MAIN)
        self._pages["color"] = page

        # Image loader
        load_card = RoundedCard(page, fill=BG_CARD, radius=14, padding=22)
        load_card.pack(fill="x", pady=(0, 12))
        self._cr_path_label = self._folder_row(
            load_card.body, "🖼", "Map Image",
            "No image loaded — click Browse to select a PNG",
            self._cr_load_image)

        # Two previews
        prev_row = tk.Frame(page, bg=BG_MAIN)
        prev_row.pack(fill="x", pady=(0, 12))
        prev_row.columnconfigure(0, weight=1)
        prev_row.columnconfigure(1, weight=1)

        # Original
        left = RoundedCard(prev_row, fill=BG_CARD, radius=14, padding=12)
        left.grid(row=0, column=0, sticky="nsew", padx=(0, 6))

        tk.Label(left.body, text="👆   CLICK TO PICK TARGET COLOR",
                 bg=BG_CARD, fg=FG_DIM,
                 font=("Segoe UI", 8, "bold")).pack(pady=(2, 8))

        self._cr_canvas = tk.Canvas(left.body, width=380, height=210,
                                     bg=BG_INPUT, highlightthickness=0,
                                     cursor="crosshair")
        self._cr_canvas.pack(fill="x")
        self._cr_canvas.bind("<Button-1>", self._cr_pick_from_image)
        draw_empty_state(self._cr_canvas, "Open an image to begin")

        # Live preview
        right = RoundedCard(prev_row, fill=BG_CARD, radius=14, padding=12)
        right.grid(row=0, column=1, sticky="nsew", padx=(6, 0))

        live_hdr = tk.Frame(right.body, bg=BG_CARD)
        live_hdr.pack(pady=(2, 8))
        tk.Label(live_hdr, text="●", bg=BG_CARD, fg=RED,
                 font=("Segoe UI", 9)).pack(side="left")
        tk.Label(live_hdr, text="  LIVE PREVIEW", bg=BG_CARD, fg=FG_DIM,
                 font=("Segoe UI", 8, "bold")).pack(side="left")

        self._cr_live_canvas = tk.Canvas(right.body, width=380, height=210,
                                          bg=BG_INPUT, highlightthickness=0)
        self._cr_live_canvas.pack(fill="x")
        draw_empty_state(self._cr_live_canvas, "Live preview will appear here")

        # Colors row
        colors = RoundedCard(page, fill=BG_CARD, radius=14, padding=18)
        colors.pack(fill="x", pady=(0, 12))

        crow = tk.Frame(colors.body, bg=BG_CARD)
        crow.pack(fill="x")

        # Target column
        tc = tk.Frame(crow, bg=BG_CARD)
        tc.pack(side="left", expand=True, fill="both")
        tk.Label(tc, text="🎯  TARGET", bg=BG_CARD, fg=FG_DIM,
                 font=("Segoe UI", 8, "bold")).pack()
        self._cr_target_swatch = ColorSwatch(tc, "#808080", size=48,
                                              on_click=self._cr_pick_target_color)
        self._cr_target_swatch.pack(pady=(6, 4))
        self._cr_target_label = tk.Label(tc, text="(128, 128, 128)",
                                          bg=BG_CARD, fg=FG_MUTED,
                                          font=("Consolas", 9))
        self._cr_target_label.pack()

        # Arrow
        tk.Label(crow, text="→", bg=BG_CARD, fg=PRIMARY,
                 font=("Segoe UI", 20, "bold")).pack(side="left", padx=18)

        # Replace column
        rc = tk.Frame(crow, bg=BG_CARD)
        rc.pack(side="left", expand=True, fill="both")
        tk.Label(rc, text="🎨  REPLACE WITH", bg=BG_CARD, fg=FG_DIM,
                 font=("Segoe UI", 8, "bold")).pack()
        self._cr_replace_swatch = ColorSwatch(rc, "#ff0000", size=48,
                                               on_click=self._cr_pick_replace_color)
        self._cr_replace_swatch.pack(pady=(6, 4))
        self._cr_replace_label = tk.Label(rc, text="(255, 0, 0)",
                                           bg=BG_CARD, fg=FG_MUTED,
                                           font=("Consolas", 9))
        self._cr_replace_label.pack()

        # Hue card
        hue = RoundedCard(page, fill=BG_CARD, radius=14, padding=16)
        hue.pack(fill="x", pady=(0, 12))
        tk.Label(hue.body, text="QUICK HUE", bg=BG_CARD, fg=FG_DIM,
                 font=("Segoe UI", 8, "bold")).pack(anchor="w", padx=2)

        self.hue_canvas = tk.Canvas(hue.body, height=22, bg=BG_CARD,
                                     highlightthickness=1,
                                     highlightbackground=BORDER_STRONG,
                                     cursor="hand2")
        self.hue_canvas.pack(fill="x", pady=(8, 2), padx=2)
        self.hue_canvas.bind("<Button-1>", self._cr_select_hue)
        self.hue_canvas.bind("<B1-Motion>", self._cr_select_hue)
        self.hue_canvas.bind("<Configure>", self._draw_hue_bar)

        # Tolerance
        tol = RoundedCard(page, fill=BG_CARD, radius=14, padding=16)
        tol.pack(fill="x", pady=(0, 12))

        tol_hdr = tk.Frame(tol.body, bg=BG_CARD)
        tol_hdr.pack(fill="x", padx=2)
        tk.Label(tol_hdr, text="TOLERANCE", bg=BG_CARD, fg=FG_DIM,
                 font=("Segoe UI", 8, "bold")).pack(side="left")
        self._cr_tol_label = tk.Label(tol_hdr, text="40", bg=BG_CARD, fg=PRIMARY,
                                        font=("Consolas", 12, "bold"))
        self._cr_tol_label.pack(side="right")

        self._cr_tolerance = tk.IntVar(value=40)
        CustomSlider(tol.body, self._cr_tolerance, from_=1, to=150,
                     height=32, on_change=self._cr_on_tol_change
                     ).pack(fill="x", pady=(8, 2), padx=2)
        tk.Label(tol.body, text="Higher values replace more colors",
                 bg=BG_CARD, fg=FG_MUTED,
                 font=("Segoe UI", 8)).pack(anchor="w", padx=2)

        # Actions
        btn_row = tk.Frame(page, bg=BG_MAIN)
        btn_row.pack(fill="x")
        PillButton(btn_row, "👁   Full Preview", self._cr_full_preview,
                   bg_color=BG_INPUT, fg_color=FG, hover_color=BORDER_STRONG,
                   width=180, height=44, radius=10,
                   font=("Segoe UI", 10, "bold")).pack(side="left", padx=(0, 10))
        PillButton(btn_row, "💾   Save Result", self._cr_save,
                   bg_color=GREEN, fg_color=BG_DEEP, hover_color=GREEN_HOVER,
                   width=180, height=44, radius=10,
                   font=("Segoe UI", 10, "bold")).pack(side="left")

    # ══════════════════════════════════════════════════════
    #  COLOR: LIVE PREVIEW LOGIC
    # ══════════════════════════════════════════════════════
    def _cr_load_image(self):
        path = filedialog.askopenfilename(
            title="Select map image",
            filetypes=[("PNG Image", "*.png"),
                       ("All Images", "*.png;*.jpg;*.jpeg;*.bmp")],
        )
        if not path:
            return
        self._set_status("Loading image…", YELLOW)
        try:
            self._cr_image_path = path
            self._cr_pil_image = Image.open(path).convert("RGBA")
            self._cr_path_label.config(text=path, fg=PRIMARY)

            preview = self._cr_pil_image.copy()
            preview.thumbnail((320, 320), Image.LANCZOS)
            self._cr_preview_src = preview

            self._cr_show_thumbnail()
            self._cr_schedule_live_preview()
            self._set_status(f"Loaded {os.path.basename(path)}", GREEN)
        except Exception as exc:
            self._set_status("Failed to load image", RED)
            messagebox.showerror("Error", f"Could not open image:\n{exc}")

    def _cr_on_original_canvas_resize(self, event=None):
        """Redraw the source preview at the actual canvas size after resizing."""
        if self._cr_pil_image is None:
            return
        if self._cr_thumb_resize_job is not None:
            try:
                self.root.after_cancel(self._cr_thumb_resize_job)
            except tk.TclError:
                pass
        try:
            self._cr_thumb_resize_job = self.root.after(90, self._cr_show_thumbnail)
        except tk.TclError:
            self._cr_thumb_resize_job = None

    def _cr_on_live_canvas_resize(self, event=None):
        """Refresh the live preview when the available display area changes."""
        if self._cr_preview_src is None:
            return
        if self._cr_live_resize_job is not None:
            try:
                self.root.after_cancel(self._cr_live_resize_job)
            except tk.TclError:
                pass
        try:
            self._cr_live_resize_job = self.root.after(120, self._cr_refresh_live_preview_after_resize)
        except tk.TclError:
            self._cr_live_resize_job = None

    def _cr_refresh_live_preview_after_resize(self):
        self._cr_live_resize_job = None
        self._cr_schedule_live_preview()

    def _cr_show_thumbnail(self):
        self._cr_thumb_resize_job = None
        if self._cr_pil_image is None:
            return
        from PIL import ImageTk

        cw = self._cr_canvas.winfo_width()
        ch = self._cr_canvas.winfo_height()
        if cw < 10:
            cw = 380
        if ch < 10:
            ch = 230

        img = self._cr_pil_image.copy()
        img.thumbnail((cw, ch), Image.LANCZOS)
        self._cr_thumb_size = img.size
        self._cr_thumb_offset = ((cw - img.size[0]) // 2,
                                  (ch - img.size[1]) // 2)
        self._cr_tk_thumb = ImageTk.PhotoImage(img)

        self._cr_canvas.delete("all")
        self._cr_canvas.create_image(
            self._cr_thumb_offset[0], self._cr_thumb_offset[1],
            anchor="nw", image=self._cr_tk_thumb)

    def _cr_pick_from_image(self, event):
        if self._cr_pil_image is None:
            return
        tx = event.x - self._cr_thumb_offset[0]
        ty = event.y - self._cr_thumb_offset[1]
        tw, th = self._cr_thumb_size
        if tx < 0 or ty < 0 or tx >= tw or ty >= th:
            return

        ow, oh = self._cr_pil_image.size
        ox = max(0, min(int(tx * ow / tw), ow - 1))
        oy = max(0, min(int(ty * oh / th), oh - 1))
        r, g, b, _ = self._cr_pil_image.getpixel((ox, oy))

        self._cr_target_rgb = (r, g, b)
        self._cr_target_swatch.set_color(f"#{r:02x}{g:02x}{b:02x}")
        self._cr_target_label.config(text=f"({r}, {g}, {b})")
        self._cr_schedule_live_preview()

    def _cr_pick_target_color(self):
        initial = "#{:02x}{:02x}{:02x}".format(*self._cr_target_rgb)
        color = colorchooser.askcolor(color=initial, title="Choose target color")
        if not color or not color[0]:
            return
        r, g, b = (int(round(c)) for c in color[0])
        self._cr_target_rgb = (r, g, b)
        self._cr_target_swatch.set_color(f"#{r:02x}{g:02x}{b:02x}")
        self._cr_target_label.config(text=f"({r}, {g}, {b})")
        self._cr_schedule_live_preview()

    def _cr_pick_replace_color(self):
        initial = "#{:02x}{:02x}{:02x}".format(*self._cr_replace_rgb)
        color = colorchooser.askcolor(color=initial, title="Choose replacement color")
        if not color or not color[0]:
            return
        r, g, b = (int(round(c)) for c in color[0])
        self._cr_set_replace_color(r, g, b)

    def _cr_set_replace_color(self, r, g, b):
        r, g, b = max(0, min(255, r)), max(0, min(255, g)), max(0, min(255, b))
        self._cr_replace_rgb = (r, g, b)
        self._cr_replace_swatch.set_color(f"#{r:02x}{g:02x}{b:02x}")
        self._cr_replace_label.config(text=f"({r}, {g}, {b})")
        self._cr_schedule_live_preview()

    def _draw_hue_bar(self, event=None):
        w = self.hue_canvas.winfo_width()
        h = self.hue_canvas.winfo_height()
        if w < 10:
            w = 400
        if h < 10:
            h = 22

        self.hue_canvas.delete("all")
        for x in range(w):
            hue = x / max(1, w - 1)
            r, g, b = (round(c * 255) for c in colorsys.hsv_to_rgb(hue, 1, 1))
            self.hue_canvas.create_line(x, 0, x, h,
                                         fill=f"#{r:02x}{g:02x}{b:02x}")

    def _cr_select_hue(self, event):
        w = self.hue_canvas.winfo_width()
        if w < 2:
            return
        hue = max(0.0, min(1.0, event.x / (w - 1)))
        r, g, b = (round(c * 255) for c in colorsys.hsv_to_rgb(hue, 1, 1))
        self._cr_set_replace_color(r, g, b)

    def _cr_on_tol_change(self, val):
        self._cr_tol_label.config(text=str(int(val)))
        self._cr_schedule_live_preview()

    def _cr_schedule_live_preview(self):
        if self._cr_preview_job is not None:
            try:
                self.root.after_cancel(self._cr_preview_job)
            except Exception:
                pass
            self._cr_preview_job = None
        try:
            self._cr_preview_job = self.root.after(110, self._cr_update_live_preview)
        except tk.TclError:
            self._cr_preview_job = None

    def _cr_update_live_preview(self):
        self._cr_preview_job = None
        if self._cr_preview_src is None:
            return

        try:
            result = self._cr_recolor_image(
                self._cr_preview_src,
                self._cr_target_rgb,
                self._cr_replace_rgb,
                float(self._cr_tolerance.get()),
            )
        except Exception:
            return

        from PIL import ImageTk

        cw = self._cr_live_canvas.winfo_width()
        ch = self._cr_live_canvas.winfo_height()
        if cw < 10:
            cw = 380
        if ch < 10:
            ch = 230

        display = result.copy()
        display.thumbnail((cw, ch), Image.LANCZOS)
        self._cr_live_tk = ImageTk.PhotoImage(display)

        self._cr_live_canvas.delete("all")
        ox = (cw - display.size[0]) // 2
        oy = (ch - display.size[1]) // 2
        self._cr_live_canvas.create_image(ox, oy, anchor="nw",
                                            image=self._cr_live_tk)

    def _cr_recolor_image(self, image, target_rgb, replace_rgb, tolerance):
        img = image.convert("RGBA")
        tr, tg, tb = target_rgb
        rr, rg, rb = replace_rgb
        dr, dg, db = (rr - tr), (rg - tg), (rb - tb)
        tol = max(1.0, float(tolerance))

        src = img.getdata()
        out = [None] * len(src)
        for i, (r, g, b, a) in enumerate(src):
            diff_r, diff_g, diff_b = r - tr, g - tg, b - tb
            dist = (diff_r * diff_r + diff_g * diff_g + diff_b * diff_b) ** 0.5
            mask = 1.0 - (dist / tol)
            if mask <= 0.0:
                out[i] = (r, g, b, a)
                continue
            if mask > 1.0:
                mask = 1.0
            nr = r + dr * mask
            ng = g + dg * mask
            nb = b + db * mask
            out[i] = (
                0 if nr < 0 else (255 if nr > 255 else int(round(nr))),
                0 if ng < 0 else (255 if ng > 255 else int(round(ng))),
                0 if nb < 0 else (255 if nb > 255 else int(round(nb))),
                a,
            )
        result = Image.new("RGBA", img.size)
        result.putdata(out)
        return result

    def _cr_save(self):
        if self._cr_pil_image is None:
            messagebox.showerror("Error", "Please load an image first.")
            return
        save_path = filedialog.asksaveasfilename(
            title="Save recolored image", defaultextension=".png",
            initialfile="map_recolored.png",
            initialdir=os.path.dirname(self._cr_image_path),
            filetypes=[("PNG Image", "*.png")],
        )
        if not save_path:
            return
        self._set_status("Applying color replacement…", YELLOW)
        self.root.update_idletasks()
        try:
            result = self._cr_recolor_image(
                self._cr_pil_image, self._cr_target_rgb,
                self._cr_replace_rgb, float(self._cr_tolerance.get()))
            result.save(save_path)
            self._set_status(f"Saved → {os.path.basename(save_path)}", GREEN)
            messagebox.showinfo("Done ✅",
                                f"Color replaced and saved!\n{os.path.basename(save_path)}")
        except Exception as exc:
            self._set_status("Save failed", RED)
            messagebox.showerror("Error", str(exc))

    def _cr_full_preview(self):
        if self._cr_pil_image is None:
            messagebox.showerror("Error", "Please load an image first.")
            return
        result = self._cr_recolor_image(
            self._cr_pil_image, self._cr_target_rgb,
            self._cr_replace_rgb, float(self._cr_tolerance.get()))
        self._cr_show_preview_window(result)

    def _cr_show_preview_window(self, result_image):
        from PIL import ImageTk

        win = tk.Toplevel(self.root)
        win.title("Preview — Recolored Map")
        win.configure(bg=BG_MAIN)
        win.geometry("920x720")
        win.minsize(500, 400)

        frame = tk.Frame(win, bg=BG_MAIN)
        frame.pack(fill="both", expand=True, padx=14, pady=(14, 6))

        hs = tk.Scrollbar(frame, orient="horizontal")
        vs = tk.Scrollbar(frame, orient="vertical")
        cv = tk.Canvas(frame, bg=BG_DEEP, highlightthickness=0,
                        xscrollcommand=hs.set, yscrollcommand=vs.set)
        hs.config(command=cv.xview)
        vs.config(command=cv.yview)
        vs.pack(side="right", fill="y")
        hs.pack(side="bottom", fill="x")
        cv.pack(side="left", fill="both", expand=True)

        display = result_image
        if max(display.size) > 1600:
            display = display.copy()
            display.thumbnail((1600, 1600), Image.LANCZOS)
        tkimg = ImageTk.PhotoImage(display)
        cv.image = tkimg
        cv.create_image(0, 0, anchor="nw", image=tkimg)
        cv.config(scrollregion=(0, 0, display.size[0], display.size[1]))

        bar = tk.Frame(win, bg=BG_MAIN)
        bar.pack(fill="x", padx=14, pady=(0, 14))

        def do_save():
            path = filedialog.asksaveasfilename(
                title="Save image", defaultextension=".png",
                initialfile="map_recolored.png",
                initialdir=os.path.dirname(self._cr_image_path) if self._cr_image_path else None,
                filetypes=[("PNG Image", "*.png")], parent=win,
            )
            if not path:
                return
            result_image.save(path)
            messagebox.showinfo("Done ✅", f"Saved!\n{os.path.basename(path)}", parent=win)
            win.destroy()

        PillButton(bar, "💾   Save", do_save,
                   bg_color=GREEN, fg_color=BG_DEEP, hover_color=GREEN_HOVER,
                   width=140, height=40, radius=9,
                   font=("Segoe UI", 10, "bold")).pack(side="left", padx=(0, 8))
        PillButton(bar, "✖   Close", win.destroy,
                   bg_color=BG_INPUT, fg_color=FG, hover_color=BORDER_STRONG,
                   width=140, height=40, radius=9,
                   font=("Segoe UI", 10, "bold")).pack(side="left")

    # ══════════════════════════════════════════════════════
    #  FOLDER SELECTORS
    # ══════════════════════════════════════════════════════
    def select_input(self):
        f = filedialog.askdirectory(title="Select Input Folder")
        if f:
            self.input_folder = f
            self.input_label.config(text=f, fg=PRIMARY)

    def select_output(self):
        f = filedialog.askdirectory(title="Select Output Folder")
        if f:
            self.output_folder = f
            self.output_label.config(text=f, fg=PRIMARY)

    def select_txd_input(self):
        f = filedialog.askdirectory(title="Select PNG Input Folder")
        if f:
            self.txd_input_folder = f
            self.txd_input_label.config(text=f, fg=PRIMARY)

    def select_txd_output(self):
        f = filedialog.askdirectory(title="Select TXD Output Folder")
        if f:
            self.txd_output_folder = f
            self.txd_output_label.config(text=f, fg=PRIMARY)

    # ══════════════════════════════════════════════════════
    #  MERGE / SPLIT
    # ══════════════════════════════════════════════════════
    def _get_tile_size(self):
        try:
            w = int(self.tile_w_var.get())
            h = int(self.tile_h_var.get())
            if w <= 0 or h <= 0:
                raise ValueError
            return w, h
        except ValueError:
            messagebox.showerror("Error", "Tile size must be positive integers.")
            return None

    def _tile_position(self, index, tw, th):
        col = index % GRID_SIZE
        row = index // GRID_SIZE
        if self.flip_vertical.get():
            row = GRID_SIZE - 1 - row
        return col * tw, row * th

    def merge_tiles(self):
        if not self.input_folder:
            messagebox.showerror("Error", "Please select an input folder.")
            return
        if not self.output_folder:
            messagebox.showerror("Error", "Please select an output folder.")
            return
        size = self._get_tile_size()
        if size is None:
            return
        tw, th = size

        self._set_status("Merging tiles…", YELLOW)
        self.root.update_idletasks()

        canvas = Image.new("RGBA", (tw * GRID_SIZE, th * GRID_SIZE))
        found = 0
        for i in range(TOTAL_TILES):
            p = os.path.join(self.input_folder, f"radar{i:02}.png")
            if not os.path.exists(p):
                continue
            img = Image.open(p)
            if img.size != (tw, th):
                img = img.resize((tw, th), Image.LANCZOS)
            x, y = self._tile_position(i, tw, th)
            canvas.paste(img, (x, y))
            found += 1

        if found == 0:
            self._set_status("No tiles found", RED)
            messagebox.showerror("Error", "No radar tiles found in input folder.")
            return

        canvas.save(os.path.join(self.output_folder, "map.png"))
        self._set_status(f"Merged {found} tiles → map.png", GREEN)
        messagebox.showinfo(
            "Done",
            f"map.png saved ({tw * GRID_SIZE}×{th * GRID_SIZE}) from {found} tiles!")

    def split_map(self):
        if not self.input_folder:
            messagebox.showerror("Error", "Please select an input folder.")
            return
        if not self.output_folder:
            messagebox.showerror("Error", "Please select an output folder.")
            return
        size = self._get_tile_size()
        if size is None:
            return
        tw, th = size

        path = filedialog.askopenfilename(
            title="Select map image", initialdir=self.input_folder,
            filetypes=[("PNG Image", "*.png")])
        if not path:
            return

        self._set_status("Splitting map…", YELLOW)
        self.root.update_idletasks()

        img = Image.open(path)
        target_w, target_h = tw * GRID_SIZE, th * GRID_SIZE
        if img.size != (target_w, target_h):
            img = img.resize((target_w, target_h), Image.LANCZOS)

        for i in range(TOTAL_TILES):
            x, y = self._tile_position(i, tw, th)
            img.crop((x, y, x + tw, y + th)).save(
                os.path.join(self.output_folder, f"radar{i:02}.png"))

        self._set_status(f"Exported {TOTAL_TILES} tiles", GREEN)
        messagebox.showinfo(
            "Done",
            f"All {TOTAL_TILES} tiles exported at {tw}×{th} to output folder!")

    # ══════════════════════════════════════════════════════
    #  TXD CONVERSION
    # ══════════════════════════════════════════════════════
    def convert_to_txd(self):
        if not self.txd_input_folder:
            messagebox.showerror("Error", "Please select a PNG input folder.")
            return
        if not self.txd_output_folder:
            messagebox.showerror("Error", "Please select a TXD output folder.")
            return

        files = sorted([
            f for f in os.listdir(self.txd_input_folder)
            if f.lower().startswith("radar") and f.lower().endswith(".png")])
        if not files:
            messagebox.showerror(
                "Error",
                f"No radar*.png files found in:\n{self.txd_input_folder}")
            return

        self._set_status(f"Converting {len(files)} files…", YELLOW)
        self.root.update_idletasks()

        converted = 0
        errors = []
        for f in files:
            try:
                name = os.path.splitext(f)[0]
                data = _make_txd(name, Image.open(
                    os.path.join(self.txd_input_folder, f)))
                with open(os.path.join(self.txd_output_folder, name + ".txd"), "wb") as fh:
                    fh.write(data)
                converted += 1
            except Exception as exc:
                errors.append(f"{f}: {exc}")

        if errors:
            self._set_status(f"{converted} converted, {len(errors)} failed", RED)
            messagebox.showwarning(
                "Completed with errors",
                f"Converted {converted} file(s).\n\nErrors:\n" + "\n".join(errors))
        else:
            self._set_status(f"Converted {converted} files → TXD", GREEN)
            messagebox.showinfo("Done",
                                f"Successfully converted {converted} PNG file(s) to TXD!")

    # ══════════════════════════════════════════════════════
    def open_telegram(self):
        webbrowser.open_new(TELEGRAM_URL)


def _enable_windows_dpi_awareness():
    """Ask Windows for per-monitor DPI rendering to avoid bitmap-stretched UI.

    This must run before creating the first Tk window. Older Windows versions
    fall back to the system-aware APIs; other platforms are left unchanged.
    """
    if os.name != "nt":
        return

    try:
        # DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 == (HANDLE)-4
        result = ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
        if result:
            return
    except Exception:
        pass

    try:
        # PROCESS_PER_MONITOR_DPI_AWARE = 2
        ctypes.windll.shcore.SetProcessDpiAwareness(2)
        return
    except Exception:
        pass

    try:
        ctypes.windll.user32.SetProcessDPIAware()
    except Exception:
        pass


# ============================================================
if __name__ == "__main__":
    try:
        _enable_windows_dpi_awareness()
        root = tk.Tk()
        app = RadarTool(root)
        root.mainloop()
    except Exception:
        tb = traceback.format_exc()
        print(tb)
        try:
            err_root = tk.Tk()
            err_root.withdraw()
            messagebox.showerror("Startup Error", tb[:2000])
            err_root.destroy()
        except Exception:
            pass
        raise