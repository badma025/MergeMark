FROM python:3.11-slim

WORKDIR /app

# Install system dependencies required for docling and vision models
RUN apt-get update && apt-get install -y \
    build-essential \
    libgl1 \
    libglib2.0-0 \
    && rm -rf /var/lib/apt/lists/*

# Copy requirements and install
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy the rest of the application
COPY main.py .

# Pre-download docling models during build time (optional but recommended for Cloud Run cold starts)
# Running python -c "from docling.document_converter import DocumentConverter; DocumentConverter()" 
# downloads the models into the image, so it doesn't happen on every cold start.
RUN python -c "from docling.document_converter import DocumentConverter; DocumentConverter()"

# Run the FastAPI app on port 8080 (Cloud Run's default expected port)
CMD ["uvicorn", "main:app", "--host", "0.0.0.0", "--port", "8080"]
