#!/usr/bin/env python3
"""Send a real X11 mouse click to the uniquely titled disposable smoke window."""
import ctypes as c
import ctypes.util
import sys

x = c.CDLL(ctypes.util.find_library('X11'))
t = c.CDLL(ctypes.util.find_library('Xtst'))
Window = c.c_ulong
Display = c.c_void_p

def bind(lib, name, result, *args):
    fn = getattr(lib, name)
    fn.restype, fn.argtypes = result, args
    return fn

open_display = bind(x, 'XOpenDisplay', Display, c.c_char_p)
root_window = bind(x, 'XDefaultRootWindow', Window, Display)
query = bind(x, 'XQueryTree', c.c_int, Display, Window, c.POINTER(Window), c.POINTER(Window), c.POINTER(c.POINTER(Window)), c.POINTER(c.c_uint))
fetch_name = bind(x, 'XFetchName', c.c_int, Display, Window, c.POINTER(c.c_char_p))
free = bind(x, 'XFree', c.c_int, c.c_void_p)
translate = bind(x, 'XTranslateCoordinates', c.c_int, Display, Window, Window, c.c_int, c.c_int, c.POINTER(c.c_int), c.POINTER(c.c_int), c.POINTER(Window))
raise_window = bind(x, 'XRaiseWindow', c.c_int, Display, Window)
focus = bind(x, 'XSetInputFocus', c.c_int, Display, Window, c.c_int, c.c_ulong)
motion = bind(t, 'XTestFakeMotionEvent', c.c_int, Display, c.c_int, c.c_int, c.c_int, c.c_ulong)
button = bind(t, 'XTestFakeButtonEvent', c.c_int, Display, c.c_uint, c.c_int, c.c_ulong)
sync = bind(x, 'XSync', c.c_int, Display, c.c_int)
close = bind(x, 'XCloseDisplay', c.c_int, Display)
display = open_display(None)
assert display, 'An X11 test display is required'
root = root_window(display)

def find(window):
    name = c.c_char_p()
    if fetch_name(display, window, c.byref(name)):
        matches = name.value == sys.argv[1].encode()
        free(name)
        if matches:
            return window
    parent, tree_root, children, count = Window(), Window(), c.POINTER(Window)(), c.c_uint()
    if not query(display, window, c.byref(tree_root), c.byref(parent), c.byref(children), c.byref(count)):
        return None
    ids = [children[i] for i in range(count.value)]
    if children:
        free(children)
    for child in ids:
        found = find(child)
        if found:
            return found
    return None

try:
    window = find(root)
    assert window, 'The disposable test window was not found'
    px, py, child = c.c_int(), c.c_int(), Window()
    assert translate(display, window, root, int(sys.argv[2]), int(sys.argv[3]), c.byref(px), c.byref(py), c.byref(child))
    raise_window(display, window)
    focus(display, window, 2, 0)
    motion(display, -1, px.value, py.value, 0)
    button(display, 1, 1, 0)
    button(display, 1, 0, 0)
    sync(display, 0)
finally:
    close(display)
