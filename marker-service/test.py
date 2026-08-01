import modal
import sys

app = modal.App("test-mineru-enums")
image = (
    modal.Image.debian_slim(python_version="3.10")
    .pip_install("magic-pdf[full]")
)

@app.function(image=image)
def get_enums():
    from magic_pdf.config.enums import SupportedPdfParseMethod
    print("Supported methods:", [e.name for e in SupportedPdfParseMethod])

