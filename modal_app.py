import os
import io
import sys
import base64
import shutil
import tempfile
import subprocess
import time
from typing import Dict, Any

import modal
import requests
from fastapi import UploadFile, File
from fastapi.responses import JSONResponse

# ---------------------------------------------------------------------------
# Modal App, Volume & Secrets Setup
# ---------------------------------------------------------------------------

APP_NAME = "marker-pdf-extraction"
app = modal.App(APP_NAME)

# Persistent volume mounted at /models to cache Surya/Marker weights
marker_models_vol = modal.Volume.from_name("marker-models-vol", create_if_missing=True)

# Container image with vLLM, Marker v2, and PyTorch
marker_image = (
    modal.Image.debian_slim(python_version="3.12")
    .apt_install(
        "libgl1",
        "libglib2.0-0",
        "libgomp1",
        "git",
    )
    .uv_pip_install(
        "vllm",
        "transformers>=5.12.1",
        "marker-pdf[full]>=2.0.0",
        "torch==2.8.0",
        "torchvision",
        "fastapi[standard]",
        "pydantic",
        "python-multipart",
        "pillow",
        "accelerate",
        "huggingface_hub",
        "pypdfium2",
        "requests",
    )
    .env({
        "HF_HOME": "/models/huggingface",
        "TORCH_HOME": "/models/torch",
        "MARKER_MODEL_PATH": "/models",
        "SURYA_INFERENCE_BACKEND": "vllm",
        "SURYA_INFERENCE_URL": "http://127.0.0.1:8000/v1",
        "SURYA_MODEL_CHECKPOINT": "datalab-to/surya-ocr-2",
        "VLLM_USE_MODELSCOPE": "False",
    })
)

# ---------------------------------------------------------------------------
# Initialization / Model Caching Function
# ---------------------------------------------------------------------------

@app.function(
    image=marker_image,
    volumes={"/models": marker_models_vol},
    secrets=[modal.Secret.from_name("huggingface-secret")],
    timeout=300,
    env={
        "SURYA_INFERENCE_BACKEND": "vllm",
        "SURYA_INFERENCE_URL": "http://127.0.0.1:8000/v1",
        "SURYA_MODEL_CHECKPOINT": "datalab-to/surya-ocr-2",
    },
)
def download_models():
    """
    Downloads and caches Marker v2 / Surya model weights into the mounted volume (/models).
    Run with: modal run modal_app.py::download_models
    """
    import os
    os.environ["HF_HOME"] = "/models/huggingface"
    os.environ["TORCH_HOME"] = "/models/torch"
    os.environ["MARKER_MODEL_PATH"] = "/models"
    os.environ["SURYA_INFERENCE_BACKEND"] = "vllm"
    os.environ["SURYA_INFERENCE_URL"] = "http://127.0.0.1:8000/v1"
    os.environ["SURYA_MODEL_CHECKPOINT"] = "datalab-to/surya-ocr-2"

    print("Initializing Marker models and downloading weights to /models...")
    from surya.settings import settings

    from marker.models import create_model_dict

    _ = create_model_dict()
    marker_models_vol.commit()
    print("Marker model weights successfully committed to volume.")
    return {"status": "success"}


# ---------------------------------------------------------------------------
# Serverless GPU Inference Service with Co-located vLLM Background Server
# ---------------------------------------------------------------------------

@app.cls(
    image=marker_image,
    gpu="a10g",
    memory=32768,
    cpu=4.0,
    volumes={"/models": marker_models_vol},
    secrets=[modal.Secret.from_name("huggingface-secret")],
    timeout=300,
    scaledown_window=300,
    env={
        "SURYA_INFERENCE_BACKEND": "vllm",
        "SURYA_INFERENCE_URL": "http://127.0.0.1:8000/v1",
        "SURYA_MODEL_CHECKPOINT": "datalab-to/surya-ocr-2",
    },
)
class MarkerExtractor:
    """
    Stateful GPU service hosting Marker v2 with a co-located background vLLM OpenAI API server.
    """

    @modal.enter()
    def setup(self):
        import os
        os.environ["HF_HOME"] = "/models/huggingface"
        os.environ["TORCH_HOME"] = "/models/torch"
        os.environ["MARKER_MODEL_PATH"] = "/models"
        os.environ["SURYA_INFERENCE_BACKEND"] = "vllm"
        os.environ["SURYA_INFERENCE_URL"] = "http://127.0.0.1:8000/v1"
        os.environ["SURYA_MODEL_CHECKPOINT"] = "datalab-to/surya-ocr-2"

        from surya.settings import settings

        # Check if vLLM background server is already running on port 8000
        health_url = "http://127.0.0.1:8000/health"
        server_ready = False
        try:
            r = requests.get(health_url, timeout=1)
            if r.status_code == 200:
                server_ready = True
        except Exception:
            pass

        if not server_ready:
            surya_model = os.environ.get("SURYA_MODEL_CHECKPOINT", "datalab-to/surya-ocr-2")
            print(f"Launching local vLLM OpenAI API server for model '{surya_model}'...")
            vllm_cmd = [
                sys.executable,
                "-m", "vllm.entrypoints.openai.api_server",
                "--model", surya_model,
                "--host", "127.0.0.1",
                "--port", "8000",
                "--gpu-memory-utilization", "0.7",
                "--trust-remote-code",
                "--max-model-len", "8192",
            ]
            self.vllm_log = open("/tmp/vllm-server.log", "w")
            self.vllm_proc = subprocess.Popen(
                vllm_cmd,
                stdout=self.vllm_log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )

            # Polling loop to wait until vLLM is healthy (HTTP 200)
            print("Waiting for vLLM server to become healthy on port 8000...")
            for attempt in range(120):
                try:
                    r = requests.get(health_url, timeout=2)
                    if r.status_code == 200:
                        server_ready = True
                        break
                except Exception:
                    pass
                time.sleep(1)

            if not server_ready:
                self.vllm_log.flush()
                try:
                    with open("/tmp/vllm-server.log", "r") as f:
                        log_content = f.read()
                except Exception:
                    log_content = "Unable to read vllm log file."
                raise RuntimeError(
                    f"vLLM server failed to start within 120s.\nServer Logs:\n{log_content}"
                )

        print("vLLM server is healthy on http://127.0.0.1:8000/v1")

        from marker.config.parser import ConfigParser
        from marker.converters.pdf import PdfConverter
        from marker.models import create_model_dict

        print("Loading Marker v2 model dictionary...")
        self.config_dict = {
            "output_format": "markdown",
            "mode": "balanced",
            "use_llm": False,
        }
        self.config_parser = ConfigParser(self.config_dict)
        self.model_dict = create_model_dict()

        print("Initializing PdfConverter with balanced mode and attached vLLM server...")
        self.converter = PdfConverter(
            config=self.config_parser.generate_config_dict(),
            artifact_dict=self.model_dict,
        )
        print("Marker v2 extractor setup complete and ready for inference.")

    @modal.fastapi_endpoint(method="POST")
    def extract(self, file: UploadFile = File(...)) -> Dict[str, Any]:
        """
        FastAPI endpoint accepting a multipart PDF file.
        Returns extracted Markdown and base64-encoded figures/diagrams.
        """
        if not file.filename.lower().endswith(".pdf"):
            return JSONResponse(
                status_code=400,
                content={"status": "error", "message": "Only PDF files are supported."},
            )

        temp_dir = tempfile.mkdtemp()
        temp_pdf_path = os.path.join(temp_dir, file.filename)

        try:
            # Save the uploaded PDF bytes to disk for Marker processing
            with open(temp_pdf_path, "wb") as buffer:
                shutil.copyfileobj(file.file, buffer)

            # Convert PDF using Marker v2 with balanced extraction mode
            from marker.output import text_from_rendered
            rendered = self.converter(temp_pdf_path)
            markdown_text, _, images = text_from_rendered(rendered)

            # Encode all extracted figures/diagrams to base64 strings
            encoded_images: Dict[str, str] = {}
            if images:
                for img_name, img_data in images.items():
                    try:
                        if hasattr(img_data, "save"):  # PIL Image
                            buf = io.BytesIO()
                            img_data.save(buf, format="PNG")
                            encoded_images[img_name] = base64.b64encode(buf.getvalue()).decode("utf-8")
                        elif isinstance(img_data, bytes):
                            encoded_images[img_name] = base64.b64encode(img_data).decode("utf-8")
                        elif isinstance(img_data, str):
                            encoded_images[img_name] = img_data
                    except Exception as img_err:
                        print(f"Warning: Failed to encode image {img_name}: {img_err}")

            return {
                "status": "success",
                "markdown": markdown_text,
                "images": encoded_images,
            }

        except Exception as e:
            print(f"Extraction error: {str(e)}")
            return JSONResponse(
                status_code=500,
                content={"status": "error", "message": f"Marker extraction failed: {str(e)}"},
            )
        finally:
            if os.path.exists(temp_dir):
                shutil.rmtree(temp_dir, ignore_errors=True)
