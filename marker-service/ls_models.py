import modal
import os

app = modal.App("ls-models")
vol = modal.Volume.from_name("mineru-models-vol")

@app.function(volumes={"/models": vol})
def ls_models():
    import magic_pdf
    import os
    pkg_dir = os.path.dirname(magic_pdf.__file__)
    os.system(f"cat {pkg_dir}/magic-pdf.template.json")
