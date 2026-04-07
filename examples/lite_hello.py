"""End-to-end Pyodide / xlwings Lite demo.

Open xlwings Lite in Excel, drop the snippet below into the Python tab
(or DevTools console), and the same three round trips run inside the
browser sandbox. Origin is set automatically by the iframe to
``https://addin.xlwings.org`` — no manual override needed.

Prereqs:
    * The launcher is running on the *same machine* as Excel.
    * The launcher's TLS cert is trusted by Windows (mkcert -install
      done once during Phase 0, or the launcher's self-issued CA
      registered after Phase 1.4-followup ships).
    * micropip can install xlpod from PyPI:
          import micropip
          await micropip.install("xlpod")
      Until the wheel is published, paste this file's body inline.

Snippet:

    import micropip
    await micropip.install("xlpod")

    import xlpod

    client = xlpod.AsyncClient()
    print(await client.health())
    await client.handshake(scopes=["fs:read"])
    print(await client.version())

That is the entire end-to-end demo on the Lite side. The protocol is
identical to the desktop client; the only difference is the transport
(``pyodide.http.pyfetch`` instead of ``httpx.AsyncClient``), which the
package autodetects from ``sys.platform``.
"""
