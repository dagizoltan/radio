FROM denoland/deno:1.41.0

WORKDIR /app

# Ensure deno user owns the working directory so it can create lockfiles
RUN chown -R deno:deno /app

USER deno

# Cache the dependencies as a layer
COPY --chown=deno:deno deno.json main.js ./

RUN deno cache main.js

# These steps will be re-run upon each file change in your working directory:
COPY --chown=deno:deno . .

EXPOSE 3000

CMD ["run", "--allow-net", "--allow-env", "--allow-read", "main.js"]
