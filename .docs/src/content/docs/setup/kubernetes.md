---
title: Kubernetes Deployment
description: Deploy Passage on Kubernetes with high availability and auto-scaling.
sidebar:
    order: 4
---

This guide covers deploying Passage on Kubernetes, from basic setups to production-grade configurations with Agones integration.

## Prerequisites

- Kubernetes cluster (1.25+)
- `kubectl` configured
- (Optional) Helm 3.x for chart-based deployment
- (Optional) Agones installed for game server discovery

## Quick Start with Helm (Recommended)

The official Helm chart is the easiest way to deploy Passage:

```sh
helm install passage oci://ghcr.io/scrayosnet/helm/passage \
  --version 0.3.0 \
  --namespace passage --create-namespace
```

To expose Passage via a cloud load balancer:

```sh
helm install passage oci://ghcr.io/scrayosnet/helm/passage \
  --version 0.3.0 \
  --namespace passage --create-namespace \
  --set service.type=LoadBalancer
```

### Helm with Custom Configuration

Create a `values.yaml` to configure Passage and enable Agones:

```yaml
# values.yaml
service:
  type: LoadBalancer

config:
  # Passage application configuration goes here
  # See Reference > Configuration for all options

rbac:
  agones:
    enabled: true
    gameserverNamespace: minecraft
```

```sh
helm install passage oci://ghcr.io/scrayosnet/helm/passage \
  --version 0.3.0 \
  --namespace passage --create-namespace \
  -f values.yaml
```

All available options are documented in [`helm/values.yaml`](https://github.com/scrayosnet/passage/blob/main/helm/values.yaml).

## Manual Deployment

### ConfigMap

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: passage-config
  namespace: passage
data:
  config.yaml: |
    address: "0.0.0.0:25565"
    timeout: 120

    rate_limiter:
      duration: 60
      limit: 60

    routes:
    - hostname: "mc.example.net"
      status:
        type: fixed
        name: "My Kubernetes Network"
        description: "\"Powered by Passage\""
      discovery:
        type: fixed_discovery
        targets:
        - identifier: "hub-1"
          address: "hub-service.minecraft:25565"
```

### Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: passage
  namespace: passage
spec:
  replicas: 2
  selector:
    matchLabels:
      app: passage
  template:
    metadata:
      labels:
        app: passage
    spec:
      containers:
      - name: passage
        image: ghcr.io/scrayosnet/passage:latest
        ports:
        - containerPort: 25565
          protocol: TCP
        resources:
          requests:
            memory: "64Mi"
            cpu: "100m"
          limits:
            memory: "256Mi"
            cpu: "500m"
        volumeMounts:
        - name: config
          mountPath: /app/config
          readOnly: true
        env:
        - name: RUST_LOG
          value: "info"
      volumes:
      - name: config
        configMap:
          name: passage-config
```

### Service

```yaml
apiVersion: v1
kind: Service
metadata:
  name: passage
  namespace: passage
spec:
  type: LoadBalancer
  externalTrafficPolicy: Local  # Preserve client IP
  ports:
  - port: 25565
    targetPort: 25565
    protocol: TCP
    name: minecraft
  selector:
    app: passage
```

Deploy:
```bash
kubectl apply -f passage-deployment.yaml
kubectl get svc passage -n passage  # Get external IP
```

## Production Configuration

### High Availability

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: passage
  namespace: passage
spec:
  replicas: 3
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 1
      maxUnavailable: 0  # Zero-downtime updates
  selector:
    matchLabels:
      app: passage
  template:
    metadata:
      labels:
        app: passage
    spec:
      affinity:
        podAntiAffinity:
          preferredDuringSchedulingIgnoredDuringExecution:
          - weight: 100
            podAffinityTerm:
              labelSelector:
                matchExpressions:
                - key: app
                  operator: In
                  values: [passage]
              topologyKey: kubernetes.io/hostname
      containers:
      - name: passage
        image: ghcr.io/scrayosnet/passage:v0.1.24
        ports:
        - containerPort: 25565
          protocol: TCP
        resources:
          requests:
            memory: "128Mi"
            cpu: "200m"
          limits:
            memory: "512Mi"
            cpu: "1000m"
        livenessProbe:
          tcpSocket:
            port: 25565
          initialDelaySeconds: 10
          periodSeconds: 30
        readinessProbe:
          tcpSocket:
            port: 25565
          initialDelaySeconds: 5
          periodSeconds: 10
        volumeMounts:
        - name: config
          mountPath: /app/config
          readOnly: true
        - name: auth-secret
          mountPath: /run/secrets
          readOnly: true
        env:
        - name: RUST_LOG
          value: "info"
        - name: AUTH_SECRET_FILE
          value: "/run/secrets/auth-secret"
      volumes:
      - name: config
        configMap:
          name: passage-config
      - name: auth-secret
        secret:
          secretName: passage-auth-secret
```

### Authentication Secret

```bash
openssl rand -base64 32 > auth_secret
kubectl create secret generic passage-auth-secret \
  --from-file=auth-secret=auth_secret \
  -n passage
rm auth_secret
```

### Resource Guidelines

| Scenario | CPU Request | CPU Limit | Memory Request | Memory Limit |
|----------|-------------|-----------|----------------|--------------|
| Small (<100 players/min) | 100m | 500m | 64Mi | 256Mi |
| Medium (<500 players/min) | 200m | 1000m | 128Mi | 512Mi |
| Large (<2000 players/min) | 500m | 2000m | 256Mi | 1Gi |

## Agones Integration

### RBAC Permissions

When using the Helm chart, set `rbac.agones.enabled: true` -- the chart creates the necessary ClusterRole and RoleBinding automatically.

For manual setup:

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: passage
  namespace: passage
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: passage
rules:
- apiGroups: [""]
  resources: ["events"]
  verbs: ["create", "patch"]
- apiGroups: ["agones.dev"]
  resources: ["gameservers"]
  verbs: ["list", "watch", "patch"]
- apiGroups: ["allocation.agones.dev"]
  resources: ["gameserverallocations"]
  verbs: ["create"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: passage
  namespace: minecraft  # Namespace where GameServers are
subjects:
- kind: ServiceAccount
  name: passage
  namespace: passage
roleRef:
  kind: ClusterRole
  name: passage
  apiGroup: rbac.authorization.k8s.io
```

### Passage Configuration for Agones

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: passage-config
  namespace: passage
data:
  config.yaml: |
    address: "0.0.0.0:25565"
    timeout: 120

    rate_limiter:
      duration: 60
      limit: 100

    routes:
    - hostname: "mc.example.net"
      status:
        type: http
        address: "http://status-service.minecraft/status"
        cache_duration: 30
      discovery:
        type: agones_discovery
        namespace: "minecraft"
        selectors:
        - matchLabels:
            game: "minecraft"
            type: "lobby"
        scheduling: "Packed"
        actions:
        - type: player_fill_strategy
          name: "fill"
          field: "players"
          max_players: 50
```

### Example GameServer Fleet

```yaml
apiVersion: "agones.dev/v1"
kind: Fleet
metadata:
  name: minecraft-lobbies
  namespace: minecraft
spec:
  replicas: 5
  template:
    metadata:
      labels:
        game: minecraft
        type: lobby
    spec:
      ports:
      - name: minecraft
        containerPort: 25565
        protocol: TCP
      counters:
        players:
          count: 0
          capacity: 50
      template:
        spec:
          containers:
          - name: minecraft
            image: itzg/minecraft-server:latest
            env:
            - name: EULA
              value: "TRUE"
            - name: TYPE
              value: "PAPER"
```

## PROXY Protocol

If your load balancer sends PROXY protocol headers:

```yaml
# In the Passage config
proxy_protocol:
  allow_v1: true
  allow_v2: true
```

AWS NLB example:
```yaml
apiVersion: v1
kind: Service
metadata:
  name: passage
  namespace: passage
  annotations:
    service.beta.kubernetes.io/aws-load-balancer-type: "nlb"
    service.beta.kubernetes.io/aws-load-balancer-proxy-protocol: "*"
spec:
  type: LoadBalancer
  externalTrafficPolicy: Local
  ports:
  - port: 25565
    targetPort: 25565
    protocol: TCP
  selector:
    app: passage
```

## OpenTelemetry

```yaml
# In the Passage config
otel:
  environment: "production"
  traces:
    address: "http://tempo.monitoring:4318/v1/traces"
  metrics:
    address: "http://mimir.monitoring:4318/v1/metrics"
```

## Horizontal Pod Autoscaling

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: passage
  namespace: passage
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: passage
  minReplicas: 2
  maxReplicas: 10
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
```

## Troubleshooting

### Pods Not Starting

```bash
kubectl get pods -n passage
kubectl logs -n passage -l app=passage
kubectl describe pod -n passage <pod-name>
```

### Can't Connect from Minecraft

```bash
kubectl get svc passage -n passage
kubectl port-forward -n passage svc/passage 25565:25565  # Test locally
```

### Agones Discovery Not Working

```bash
# Check RBAC permissions
kubectl auth can-i watch gameservers \
  --as=system:serviceaccount:passage:passage \
  -n minecraft

# Check GameServers exist
kubectl get gameservers -n minecraft

# Check Passage logs
kubectl logs -n passage -l app=passage | grep agones
```

## Best Practices

- Use **specific image tags** (not `latest`) for reproducible deployments
- Run at least **2 replicas** with pod anti-affinity for high availability
- Use `externalTrafficPolicy: Local` to preserve client IP addresses
- Store the auth secret in a Kubernetes Secret, not in the ConfigMap
- Enable rate limiting in production
- Configure liveness and readiness probes
- Set appropriate resource requests and limits
- Use RBAC with minimal permissions
