import os
import shutil
import tempfile
from fastapi import FastAPI, UploadFile, File
from fastapi.responses import JSONResponse
from docling.document_converter import DocumentConverter

app = FastAPI(title="Docling PDF Extraction Microservice")

from docling.datamodel.pipeline_options import PdfPipelineOptions
from docling.document_converter import InputFormat, PdfFormatOption
from docling_core.types.doc.base import ImageRefMode

# Disable OCR since A-Level past papers are digital PDFs.
# This makes extraction on CPU 5x to 10x faster!
pipeline_options = PdfPipelineOptions()
pipeline_options.do_ocr = False
pipeline_options.do_table_structure = True
pipeline_options.generate_picture_images = True

converter = DocumentConverter(
    format_options={
        InputFormat.PDF: PdfFormatOption(pipeline_options=pipeline_options)
    }
)

@app.post("/extract")
async def extract_pdf(file: UploadFile = File(...)):
    if not file.filename.lower().endswith('.pdf'):
        return JSONResponse(
            status_code=200, 
            content={"status": "error", "message": "Only PDF files are supported."}
        )
        
    temp_dir = tempfile.mkdtemp()
    temp_path = os.path.join(temp_dir, file.filename)
    
    try:
        # Save the uploaded file temporarily
        with open(temp_path, "wb") as buffer:
            shutil.copyfileobj(file.file, buffer)
            
        # Convert the document using Docling
        result = converter.convert(temp_path)
        markdown = result.document.export_to_markdown(image_mode=ImageRefMode.EMBEDDED)
        
        return {
            "status": "success",
            "markdown": markdown
        }
    except Exception as e:
        return JSONResponse(
            status_code=200,
            content={"status": "error", "message": f"Extraction failed: {str(e)}"}
        )
    finally:
        # Cleanup temporary files
        if os.path.exists(temp_dir):
            shutil.rmtree(temp_dir)

if __name__ == "__main__":
    import uvicorn
    # Run the server on port 8080 as expected by the Rust backend
    uvicorn.run("main:app", host="0.0.0.0", port=8080, reload=True)
