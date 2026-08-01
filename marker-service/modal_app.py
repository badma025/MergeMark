import base64
import os
import tempfile
import json
import modal
from fastapi import FastAPI, UploadFile, File
from fastapi.responses import JSONResponse

app = modal.App("mineru-pdf-service")

# Mount a persistent volume to store the models
vol = modal.Volume.from_name("mineru-models-vol", create_if_missing=True)

# Image config for MinerU (Magic-PDF)
image = (
    modal.Image.debian_slim(python_version="3.10")
    .apt_install("libgl1-mesa-glx", "libglib2.0-0", "wget")
    # Install magic-pdf[full] and the required dependencies
    .pip_install(
        "magic-pdf[full]",
        "huggingface-hub",
        "fastapi[standard]"
    )
    # Detectron2 requires a custom wheel repository
    .run_commands("pip install detectron2 --extra-index-url https://myhloli.github.io/wheels/")
)

web_app = FastAPI(title="MinerU PDF Extraction Microservice")

def write_mineru_config():
    """Write the magic-pdf.json configuration pointing models-dir to the volume."""
    config_path = os.path.expanduser("~/magic-pdf.json")
    if not os.path.exists(config_path):
        config_content = {
            "models-dir": "/models/models"
        }
        with open(config_path, "w") as f:
            json.dump(config_content, f)

@app.function(image=image, volumes={"/models": vol}, timeout=1800)
def init_models():
    """
    Downloads models to the persistent volume if they don't exist
    and writes the magic-pdf.json configuration file.
    Runs once manually or during build setup.
    """
    write_mineru_config()
    
    # Download models if they are not already cached
    from huggingface_hub import snapshot_download
    
    print("Downloading/Verifying MinerU weights to /models volume...")
    snapshot_download(repo_id="opendatalab/PDF-Extract-Kit", local_dir="/models", local_dir_use_symlinks=False)
    
    # Fix for MFD YOLO weights path mismatch
    mfd_dir = "/models/models/MFD"
    yolo_dir = os.path.join(mfd_dir, "YOLO")
    weights_pt = os.path.join(mfd_dir, "weights.pt")
    yolo_pt = os.path.join(yolo_dir, "yolo_v8_ft.pt")
    
    if os.path.exists(weights_pt) and not os.path.exists(yolo_pt):
        os.makedirs(yolo_dir, exist_ok=True)
        os.symlink(weights_pt, yolo_pt)
        print("Created symlink for YOLO weights")
        
    # Fix for MFR unimernet path mismatch in 1.3
    mfr_dir = "/models/models/MFR"
    unimer_small = os.path.join(mfr_dir, "unimernet_small")
    unimer_2503 = os.path.join(mfr_dir, "unimernet_hf_small_2503")
    
    if os.path.exists(unimer_small) and not os.path.exists(unimer_2503):
        os.symlink(unimer_small, unimer_2503)
        print("Created symlink for unimernet_hf_small_2503")
        
    # Transformers expects pytorch_model.bin but the repo provides pytorch_model.pth
    pth_file = os.path.join(unimer_small, "pytorch_model.pth")
    bin_file = os.path.join(unimer_small, "pytorch_model.bin")
    if os.path.exists(pth_file) and not os.path.exists(bin_file):
        os.symlink(pth_file, bin_file)
        print("Created symlink for pytorch_model.bin")
        
    print("Model weights ready!")
    
    vol.commit()

@web_app.on_event("startup")
def startup_event():
    # Ensure config exists for the web app container
    write_mineru_config()
@app.function(image=image, volumes={"/models": vol})
def inspect_paths():
    import os
    print("Testing if python can read config:")
    config_path = "/models/models/MFR/unimernet_hf_small_2503/config.json"
    bin_path = "/models/models/MFR/unimernet_hf_small_2503/pytorch_model.bin"
    print("config.json exists:", os.path.exists(config_path))
    print("pytorch_model.bin exists:", os.path.exists(bin_path))
    print("pytorch_model.bin isfile:", os.path.isfile(bin_path))

@web_app.post("/extract")
async def extract_pdf(file: UploadFile = File(...)):
    if not file.filename.lower().endswith('.pdf'):
        return JSONResponse(
            status_code=200, 
            content={"status": "error", "message": "Only PDF files are supported."}
        )
        
    temp_dir = tempfile.mkdtemp()
    
    try:
        pdf_bytes = await file.read()
            
        # MinerU Pipeline
        from magic_pdf.tools.common import do_parse
        from magic_pdf.config.enums import SupportedPdfParseMethod
        import magic_pdf.model as model_config
        
        # Enable model usage
        model_config.__use_inside_model__ = True
        
        pdf_name = "paper"
        
        do_parse(
            output_dir=temp_dir,
            pdf_file_name=pdf_name,
            pdf_bytes_or_dataset=pdf_bytes,
            model_list=[],
            parse_method=SupportedPdfParseMethod.OCR.value,
            f_dump_md=True,
            f_dump_middle_json=False,
            f_dump_model_json=False,
            f_dump_orig_pdf=False,
            f_dump_content_list=False,
        )
        
        # The output path will be temp_dir/paper/ocr/
        md_dir = os.path.join(temp_dir, pdf_name, "ocr")
        md_file = os.path.join(md_dir, f"{pdf_name}.md")
        image_dir = os.path.join(md_dir, "images")
        
        if not os.path.exists(md_file):
            raise Exception("Parsing succeeded but markdown file was not created.")
            
        with open(md_file, "r", encoding="utf-8") as f:
            md_content = f.read()
        
        # Format the images payload (Base64)
        images_list = []
        if os.path.exists(image_dir):
            for filename in os.listdir(image_dir):
                filepath = os.path.join(image_dir, filename)
                if os.path.isfile(filepath):
                    with open(filepath, "rb") as f:
                        b64_str = base64.b64encode(f.read()).decode('utf-8')
                        images_list.append({
                            "filename": filename,
                            "base64": b64_str
                        })
        
        return {
            "status": "success",
            "markdown": md_content,
            "images": images_list
        }
    except Exception as e:
        import traceback
        return JSONResponse(
            status_code=200,
            content={"status": "error", "message": f"Extraction failed: {str(e)}\n{traceback.format_exc()}"}
        )
    finally:
        import shutil
        if os.path.exists(temp_dir):
            shutil.rmtree(temp_dir)

# A10G GPU for MinerU with 300 seconds timeout buffer
@app.function(image=image, volumes={"/models": vol}, gpu="a10g", timeout=300)
@modal.asgi_app()
def asgi_app():
    return web_app
