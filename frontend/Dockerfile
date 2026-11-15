# Stage 1: Build static assets
FROM node:22-alpine AS builder
WORKDIR /app

# Copy root package manifests (Tauri & workspace deps)
COPY package.json package-lock.json ./
RUN npm ci

# Copy the frontend source and Vite configuration
COPY . ./

# Build the main and session frontends with shared resources
RUN npm run build

# Stage 2: Serve via Nginx
FROM nginx:alpine AS session
COPY --from=builder /app/dist/session /usr/share/nginx/html
EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]

FROM nginx:alpine AS main
COPY --from=builder /app/dist/main /usr/share/nginx/html
EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]