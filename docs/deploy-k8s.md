# Kubernetes

## Namespace

```yaml
# namespace.yaml
apiVersion: v1
kind: Namespace
metadata:
  name: rshs
```

## Persistent Volume Claim

Data PVC for the served files:

```yaml
# pvc-data.yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: rshs-data
  namespace: rshs
spec:
  accessModes:
    - ReadWriteOnce
  resources:
    requests:
      storage: 10Gi
```

## Authentication

Two approaches for managing credentials in Kubernetes.

### Approach A: Shadow File via Secret (recommended)

Generate the shadow file locally with `openssl`, then store it as a Secret.

```sh
# Generate SHA-512 crypt hashes (compatible with rshs shadow format)
openssl passwd -6 "secret123"   # → $6$xxxxxxxx$yyyyyyyyyyyyyyyy...
openssl passwd -6 "public"       # → $6$aaaaaaaa$bbbbbbbbbbbbbb...

# Create shadow file
echo "admin:\$6\$xxxxxxxx\$yyyyyyyyyyyyyyyy..." > shadow
echo "viewer:\$6\$aaaaaaaa\$bbbbbbbbbbbbbb..." >> shadow

# Create Secret from the shadow file
kubectl create secret generic rshs-shadow --from-file=shadow -n rshs
```

The Deployment mounts this Secret as `readOnly: true` at `/etc/rshs/shadow`.
No PVC, no `-W` flag needed — all credentials live in the encrypted shadow
file. Update credentials by recreating the Secret and rolling the Deployment.

### Approach B: Environment Variable via Secret

Simplest approach — pass credentials directly via `RSHS_USERS` env var.

```yaml
# secret-auth.yaml
apiVersion: v1
kind: Secret
metadata:
  name: rshs-auth
  namespace: rshs
stringData:
  RSHS_USERS: "admin:secret123;viewer:public"
```

No shadow file, no PVC, no `-W` flag. Just inject the Secret and the server
validates against the env var value at runtime.

## Deployment

### Basic (no authentication)

```yaml
# deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: rshs
  namespace: rshs
spec:
  replicas: 1
  selector:
    matchLabels:
      app: rshs
  template:
    metadata:
      labels:
        app: rshs
    spec:
      containers:
        - name: rshs
          image: mogeko/rshs:latest
          ports:
            - containerPort: 8080
          volumeMounts:
            - name: data
              mountPath: /mnt/data
          livenessProbe:
            httpGet:
              path: /
              port: 8080
            initialDelaySeconds: 10
            periodSeconds: 30
          readinessProbe:
            httpGet:
              path: /
              port: 8080
            initialDelaySeconds: 5
            periodSeconds: 10
          resources:
            requests:
              memory: "32Mi"
              cpu: "50m"
            limits:
              memory: "128Mi"
              cpu: "500m"
      volumes:
        - name: data
          persistentVolumeClaim:
            claimName: rshs-data
```

### With Auth (Approach A: shadow file)

```yaml
# deployment-auth-shadow.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: rshs
  namespace: rshs
spec:
  replicas: 1
  selector:
    matchLabels:
      app: rshs
  template:
    metadata:
      labels:
        app: rshs
    spec:
      containers:
        - name: rshs
          image: mogeko/rshs:latest
          ports:
            - containerPort: 8080
          env:
            - name: RSHS_SHADOW_FILE
              value: "ro:/etc/rshs/shadow"
          volumeMounts:
            - name: data
              mountPath: /mnt/data
            - name: shadow
              mountPath: /etc/rshs
              readOnly: true
          livenessProbe:
            httpGet:
              path: /
              port: 8080
            initialDelaySeconds: 10
            periodSeconds: 30
          readinessProbe:
            httpGet:
              path: /
              port: 8080
            initialDelaySeconds: 5
            periodSeconds: 10
          resources:
            requests:
              memory: "32Mi"
              cpu: "50m"
            limits:
              memory: "128Mi"
              cpu: "500m"
      volumes:
        - name: data
          persistentVolumeClaim:
            claimName: rshs-data
        - name: shadow
          secret:
            secretName: rshs-shadow
```

### With Auth (Approach B: env var)

```yaml
# deployment-auth-env.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: rshs
  namespace: rshs
spec:
  replicas: 1
  selector:
    matchLabels:
      app: rshs
  template:
    metadata:
      labels:
        app: rshs
    spec:
      containers:
        - name: rshs
          image: mogeko/rshs:latest
          ports:
            - containerPort: 8080
          envFrom:
            - secretRef:
                name: rshs-auth
          volumeMounts:
            - name: data
              mountPath: /mnt/data
          livenessProbe:
            httpGet:
              path: /
              port: 8080
            initialDelaySeconds: 10
            periodSeconds: 30
          readinessProbe:
            httpGet:
              path: /
              port: 8080
            initialDelaySeconds: 5
            periodSeconds: 10
          resources:
            requests:
              memory: "32Mi"
              cpu: "50m"
            limits:
              memory: "128Mi"
              cpu: "500m"
      volumes:
        - name: data
          persistentVolumeClaim:
            claimName: rshs-data
```

## Service

```yaml
# service.yaml
apiVersion: v1
kind: Service
metadata:
  name: rshs
  namespace: rshs
spec:
  selector:
    app: rshs
  ports:
    - port: 80
      targetPort: 8080
```

## Ingress

```yaml
# ingress.yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: rshs
  namespace: rshs
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
spec:
  ingressClassName: nginx
  rules:
    - host: files.example.com
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: rshs
                port:
                  number: 80
  tls:
    - hosts:
        - files.example.com
      secretName: rshs-tls
```

## Deploy

### Approach A: Shadow file

```sh
kubectl apply -f namespace.yaml
kubectl apply -f pvc-data.yaml
kubectl apply -f deployment-auth-shadow.yaml
kubectl apply -f service.yaml
kubectl apply -f ingress.yaml
```

The shadow Secret is created separately before the Deployment (see Approach A
instructions above). Update credentials by recreating the Secret with a new
shadow file and rolling the Deployment.

### Approach B: Environment variable

```sh
kubectl apply -f namespace.yaml
kubectl apply -f pvc-data.yaml
kubectl apply -f secret-auth.yaml
kubectl apply -f deployment-auth-env.yaml
kubectl apply -f service.yaml
kubectl apply -f ingress.yaml
```

Update credentials by modifying `secret-auth.yaml` and reapplying; the
Deployment will pick up changes on the next restart.
