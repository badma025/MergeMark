import modal
import os
from fastapi import FastAPI, UploadFile, File
import json
import logging

app = modal.App("mineru-pdf-service")
web_app = FastAPI(title="MinerU PDF Extraction API")
vol = modal.Volume.from_name("mineru-models-vol", create_if_missing=True)

# Configure logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

image = (
    modal.Image.debian_slim(python_version="3.10")
    .apt_install("libgl1", "libglib2.0-0")
    .pip_install(
        "magic-pdf[full]==1.3.12",
        "huggingface_hub",
        "fastapi[standard]",
        "python-multipart",
        "timm",  # Required for LayoutLMv3
        "einops",  # Required for transformer models
    )
)

def write_mineru_config():
    """Write magic-pdf config file to the correct location"""
    config_path = os.path.expanduser("~/magic-pdf.json")
    config_content = {
        "models-dir": "/models/models",
        "device-mode": "cuda",
        "layout-config": {
            "model": "layoutlmv3"
        },
        "formula-config": {
            "mfd_model": "yolo_v8",
            "mfr_model": "unimernet_hf_small_2503"
        }
    }
    
    with open(config_path, "w") as f:
        json.dump(config_content, f, indent=2)
    
    logger.info(f"Config written to {config_path}")
    return config_path

def fix_model_paths():
    """Create all necessary symlinks for magic-pdf v1.3.12"""
    models_base = "/models/models"
    
    # 1. Fix MFD YOLO weights path
    mfd_dir = os.path.join(models_base, "MFD")
    yolo_dir = os.path.join(mfd_dir, "YOLO")
    weights_pt = os.path.join(mfd_dir, "weights.pt")
    yolo_pt = os.path.join(yolo_dir, "yolo_v8_ft.pt")
    
    if os.path.exists(weights_pt) and not os.path.exists(yolo_pt):
        os.makedirs(yolo_dir, exist_ok=True)
        os.symlink(weights_pt, yolo_pt)
        logger.info("Created symlink: YOLO weights")
    
    # 2. Fix MFR unimernet directory name
    mfr_dir = os.path.join(models_base, "MFR")
    unimer_small = os.path.join(mfr_dir, "unimernet_small")
    unimer_2503 = os.path.join(mfr_dir, "unimernet_hf_small_2503")
    
    if os.path.exists(unimer_small) and not os.path.exists(unimer_2503):
        os.symlink(unimer_small, unimer_2503)
        logger.info("Created symlink: unimernet_hf_small_2503")
    
    # 3. Fix transformers bin file
    pth_file = os.path.join(unimer_small, "pytorch_model.pth")
    bin_file = os.path.join(unimer_small, "pytorch_model.bin")
    if os.path.exists(pth_file) and not os.path.exists(bin_file):
        os.symlink(pth_file, bin_file)
        logger.info("Created symlink: pytorch_model.bin")
    
    # 4. Ensure config.json exists for transformers
    config_json = os.path.join(unimer_small, "config.json")
    if not os.path.exists(config_json):
        # Create minimal config.json if missing
        config = {
            "model_type": "unimernet",
            "architectures": ["UniMERNet"]
        }
        with open(config_json, "w") as f:
            json.dump(config, f)
        logger.info("Created config.json for unimernet")
    
    # 5. Check for LayoutLM model
    layout_dir = os.path.join(models_base, "Layout")
    if os.path.exists(layout_dir):
        logger.info(f"Layout model directory exists: {layout_dir}")
        # List contents for debugging
        for item in os.listdir(layout_dir):
            logger.info(f"  - {item}")
    
    logger.info("All model path fixes applied")

@app.function(
    image=image, 
    volumes={"/models": vol}, 
    timeout=1800,
    secrets=[]
)
def init_models():
    """Download and prepare all model weights"""
    from huggingface_hub import snapshot_download
    
    logger.info("Downloading/Verifying MinerU weights to /models volume...")
    
    # Download the model weights
    snapshot_download(
        repo_id="opendatalab/PDF-Extract-Kit",
        local_dir="/models",
        local_dir_use_symlinks=False,
        ignore_patterns=["*.md", "*.txt"]  # Skip documentation files
    )
    
    # Apply all path fixes
    fix_model_paths()
    
    # Commit the volume to persist changes
    vol.commit()
    
    logger.info("Model weights ready and committed!")
    
    # Verify the structure
    logger.info("\nVerifying model structure:")
    for root, dirs, files in os.walk("/models/models"):
        level = root.replace("/models/models", "").count(os.sep)
        indent = " " * 2 * level
        logger.info(f"{indent}{os.path.basename(root)}/")
        subindent = " " * 2 * (level + 1)
        for file in files[:5]:  # Show first 5 files
            logger.info(f"{subindent}{file}")
        if len(files) > 5:
            logger.info(f"{subindent}... and {len(files) - 5} more files")

# Global variable to track if models are initialized
models_initialized = False

@app.function(
    image=image, 
    volumes={"/models": vol}, 
    gpu="T4",
    timeout=300
)
@modal.asgi_app()
def asgi_app():
    """ASGI app with model initialization"""
    global models_initialized
    
    # Write config file
    config_path = write_mineru_config()
    
    # Verify config was written
    if not os.path.exists(config_path):
        raise RuntimeError(f"Failed to write config to {config_path}")
    
    # Apply path fixes (in case volume was reset)
    fix_model_paths()
    
    # Verify critical files exist
    required_files = [
        "/models/models/MFD/YOLO/yolo_v8_ft.pt",
        "/models/models/MFR/unimernet_hf_small_2503/pytorch_model.bin",
    ]
    
    for file_path in required_files:
        if not os.path.exists(file_path):
            raise RuntimeError(f"Required model file missing: {file_path}")
    
    models_initialized = True
    logger.info("ASGI app initialized with models")
    
    return web_app

@web_app.post("/extract")
async def extract_pdf(file: UploadFile = File(...)):
    """Extract content from PDF using MinerU"""
    global models_initialized
    
    if not models_initialized:
        return {"error": "Models not initialized"}
    
    if not file.filename.lower().endswith('.pdf'):
        return {"error": "File must be a PDF"}
    
    logger.info(f"Processing PDF: {file.filename}")
    
    try:
        pdf_bytes = await file.read()
        output_dir = "/tmp/magic_pdf_output"
        os.makedirs(output_dir, exist_ok=True)
        
        # Import here to ensure config is loaded
        from magic_pdf.tools.common import do_parse
        
        logger.info("Starting extraction pipeline...")
        
        do_parse(
            output_dir=output_dir,
            pdf_file_name=file.filename,
            pdf_bytes_or_dataset=pdf_bytes,
            p_lang_list=[],
            parse_method="ocr",
            debug_able=False
        )
        
        logger.info("Extraction completed successfully")
        
        # Check output
        output_files = os.listdir(output_dir)
        logger.info(f"Output files: {output_files}")
        
        return {
            "status": "success",
            "message": "Extraction completed",
            "output_dir": output_dir,
            "files": output_files
        }
        
    except Exception as e:
        logger.error(f"Extraction failed: {str(e)}", exc_info=True)
        return {
            "status": "error",
            "message": str(e),
            "error_type": type(e).__name__
        }
