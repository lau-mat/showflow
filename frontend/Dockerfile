# Stage 1: Build static assets
FROM node:22-alpine AS builder
WORKDIR /app

# Copy root package manifests (Tauri & workspace deps)
COPY package.json package-lock.json ./
RUN npm ci

# Copy the frontend source and Vite configuration
COPY . ./

# Build frontend/main into the repository-root dist directory
RUN npm run build

# Stage 2: Serve via Nginx
FROM nginx:alpine
COPY --from=builder /app/dist /usr/share/nginx/html
EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]