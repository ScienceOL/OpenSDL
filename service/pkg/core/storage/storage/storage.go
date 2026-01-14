package storage

import (
	// 外部依赖
	"context"
	"path"
	"strings"
	"time"

	oss "github.com/aliyun/aliyun-oss-go-sdk/oss"
	uuid "github.com/gofrs/uuid/v5"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	storage "github.com/scienceol/opensdl/service/pkg/core/storage"
	auth "github.com/scienceol/opensdl/service/pkg/middleware/auth"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	model "github.com/scienceol/opensdl/service/pkg/model"
	storageTokenRepo "github.com/scienceol/opensdl/service/pkg/repo/storage_token"
)

type StorageService struct {
	storageTokenRepo repo.StorageTokenRepo
	ossClient        *oss.Client
	bucket           *oss.Bucket
}

func NewStorageService() storage.StorageService {
	storageConf := config.Global().Dynamic().Storage

	client, err := oss.New(storageConf.OssEndpoint, storageConf.OssAccessKeyID, storageConf.OssAccessKeySecret)
	if err != nil {
		logger.Errorf(context.Background(), "failed to create OSS client: %v\n", err)
		panic(err)
	}

	bucket, err := client.Bucket(storageConf.OssBucketName)
	if err != nil {
		logger.Errorf(context.Background(), "failed to create OSS bucket: %v\n", err)
		panic(err)
	}

	return &StorageService{
		storageTokenRepo: storageTokenRepo.New(),
		ossClient:        client,
		bucket:           bucket,
	}
}

// GenerateStorageToken generates a pre-signed URL for OSS upload
func (s *StorageService) GenerateStorageToken(ctx context.Context, req *storage.GetStorageTokenReq) (*storage.GetStorageTokenResp, error) {
	// Get Storage configuration
	storageConf := config.Global().Dynamic().Storage

	labUser := auth.GetCurrentUser(ctx)
	if labUser == nil {
		return nil, code.UnLogin.WithMsg("get current user failed, please login first")
	}

	// Generate a unique object key for OSS
	randomUUID := uuid.Must(uuid.NewV4()).String()

	var objectKey string
	userPath := strings.Trim(req.SubPath, "/")

	if userPath != "" {
		// Format: scene/user_path/uuid/filename
		objectKey = path.Join(string(req.Scene), userPath, randomUUID, req.FileName)
	} else {
		// Format: scene/uuid/filename
		objectKey = path.Join(string(req.Scene), randomUUID, req.FileName)
	}

	// Calculate expiration time
	tokenTTL := storageConf.TokenTTL
	if tokenTTL <= 0 {
		tokenTTL = 1
	} // Default to 1 hour if not set

	expiredDuration := time.Duration(tokenTTL) * time.Hour
	expiresAt := time.Now().Add(expiredDuration).Unix()

	// Generate pre-signed URL for PUT request
	signedURL, err := s.bucket.SignURL(objectKey, oss.HTTPPut, int64(expiredDuration.Seconds()))
	if err != nil {
		logger.Errorf(ctx, "failed to sign URL: %v\n", err)
		return nil, code.OssGetSignErr
	}

	// Save metadata to the database (optional, but good for tracking)
	storageToken := &model.StorageToken{
		Token:  signedURL, // Storing the URL for reference
		Path:   objectKey,
		Scene:  string(req.Scene),
		UserID: labUser.ID,
	}
	if err := s.storageTokenRepo.Create(ctx, storageToken); err != nil {
		logger.Errorf(ctx, "failed to save storage token to database: %v\n", err)
		return nil, code.OssTokenSaveErr
	}

	// Return the response
	resp := &storage.GetStorageTokenResp{
		URL:     signedURL,
		Path:    objectKey,
		Expires: expiresAt,
	}

	return resp, nil
}
