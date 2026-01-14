package storage

import (
	"context"
)

type StorageService interface {
	GenerateStorageToken(ctx context.Context, req *GetStorageTokenReq) (*GetStorageTokenResp, error)
}
