FROM node:22-bookworm-slim
# openssl generates the self-signed certificate when PUBLIC_URL is https://.
RUN apt-get update \
    && apt-get install -y --no-install-recommends openssl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY . .
RUN npm run build
EXPOSE 80
CMD ["node", "server.mjs"]
