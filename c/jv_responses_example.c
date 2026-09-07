/*
 * Structured Responses API example. The legacy example is included with its
 * entry point renamed so both examples share the same reviewed URL, TLS,
 * credential, JSON-size, and password-handling primitives without copying
 * security-sensitive transport code.
 */
#define main jv_legacy_jobs_example_main
#include "jv_api_example.c"
#undef main

struct responses_options {
  const char *question;
  const char *base_url;
  const char *username;
  double poll_interval;
  double wait_timeout;
  bool print_json;
  bool tool_demo;
  bool help;
};

static int response_options(int argc, char **argv,
                            struct responses_options *options,
                            struct api_error *error) {
  memset(options, 0, sizeof(*options));
  options->base_url = getenv("JV_API_BASE_URL");
  options->username = getenv("JV_API_USERNAME");
  options->base_url = options->base_url == NULL ? DEFAULT_BASE_URL : options->base_url;
  options->username = options->username == NULL ? DEFAULT_USERNAME : options->username;
  options->poll_interval = 3.0;
  options->wait_timeout = 3600.0;
  if (argc < 2) return set_error(error, "A question is required. Use --help for usage.");
  if (strcmp(argv[1], "--help") == 0) { options->help = true; return 0; }
  options->question = argv[1];
  for (int index = 2; index < argc; ++index) {
    const char *name = argv[index];
    if (strcmp(name, "--json") == 0) { options->print_json = true; continue; }
    if (strcmp(name, "--tool-demo") == 0) { options->tool_demo = true; continue; }
    if (++index >= argc) return set_error(error, "%s requires a value.", name);
    const char *value = argv[index];
    if (strcmp(name, "--base-url") == 0) options->base_url = value;
    else if (strcmp(name, "--username") == 0) options->username = value;
    else if (strcmp(name, "--poll-interval") == 0) {
      if (positive_number(value, name, &options->poll_interval, error) != 0) return -1;
    } else if (strcmp(name, "--wait-timeout") == 0) {
      if (positive_number(value, name, &options->wait_timeout, error) != 0) return -1;
    } else return set_error(error, "Unknown option: %s", name);
  }
  if (strspn(options->question, " \t\r\n") == strlen(options->question))
    return set_error(error, "Question text must not be empty.");
  return 0;
}

static void responses_usage(const char *program) {
  printf("Usage: %s QUESTION [options]\n\n", program);
  puts("--base-url URL              Override the API origin");
  puts("--username USERNAME         Override the default username");
  puts("--poll-interval SECONDS     Poll interval; default: 3");
  puts("--wait-timeout SECONDS      Local wait timeout; default: 3600");
  puts("--json                      Print complete structured response JSON");
  puts("--tool-demo                 Run one harmless client-side tool round");
  puts("--help                      Show this help");
}

static int valid_response(json_object *value, const char *expected_id,
                          struct api_error *error) {
  const char *id = json_string_field(value, "id");
  const char *object = json_string_field(value, "object");
  const char *status = json_string_field(value, "status");
  json_object *output = NULL;
  if (id == NULL || *id == '\0' || object == NULL || strcmp(object, "response") != 0 ||
      status == NULL ||
      (strcmp(status, "queued") != 0 && strcmp(status, "in_progress") != 0 &&
       strcmp(status, "completed") != 0 && strcmp(status, "failed") != 0) ||
      !json_object_object_get_ex(value, "output", &output) ||
      !json_object_is_type(output, json_type_array))
    return set_error(error, "The API returned an invalid structured response.");
  if (expected_id != NULL && strcmp(id, expected_id) != 0)
    return set_error(error, "The polling response ID changed unexpectedly.");
  const size_t count = json_object_array_length(output);
  if (strcmp(status, "completed") == 0 ? count != 1U : count != 0U)
    return set_error(error, "The response output does not match its status.");
  return 0;
}

static void make_idempotency_key(char *buffer, size_t size) {
#ifdef _WIN32
  const unsigned long process = (unsigned long)GetCurrentProcessId();
#else
  const unsigned long process = (unsigned long)getpid();
#endif
  const unsigned long long ticks =
      (unsigned long long)(monotonic_seconds() * 1000000000.0);
  (void)snprintf(buffer, size, "jv-example-%llx-%lx", ticks, process);
}

static json_object *create_response(struct api_client *client,
                                    json_object *request,
                                    struct api_error *error) {
  struct http_response response;
  memset(&response, 0, sizeof(response));
  CURL *curl = curl_easy_init();
  struct curl_slist *headers = NULL;
  json_object *payload = NULL;
  char key[96];
  char key_header[128];
  make_idempotency_key(key, sizeof(key));
  (void)snprintf(key_header, sizeof(key_header), "Idempotency-Key: %s", key);
  if (curl == NULL) { (void)set_error(error, "Could not create an HTTP request."); return NULL; }
  const char *body = json_object_to_json_string_ext(request, JSON_C_TO_STRING_PLAIN);
  if (configure_curl(curl, client, "/v1/responses", &response, error) != 0 ||
      build_headers(client, true, true, false, &headers, error) != 0 ||
      append_header(&headers, key_header, error) != 0 ||
      curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers) != CURLE_OK ||
      curl_easy_setopt(curl, CURLOPT_POST, 1L) != CURLE_OK ||
      curl_easy_setopt(curl, CURLOPT_POSTFIELDS, body) != CURLE_OK ||
      curl_easy_setopt(curl, CURLOPT_POSTFIELDSIZE_LARGE, (curl_off_t)strlen(body)) != CURLE_OK) {
    if (*error->message == '\0') (void)set_error(error, "Could not prepare the response submission.");
    goto cleanup;
  }
  if (perform_request(curl, &response, "Response submission is uncertain.", error) != 0) {
    (void)set_error(error,
                    "Response submission is uncertain. Reconcile account state before retrying; reuse idempotency key %s.",
                    key);
    goto cleanup;
  }
  if (response.status != 200L && response.status != 202L) {
    (void)safe_http_error(&response, error);
    goto cleanup;
  }
  payload = parse_json_body(&response, error);
  if (payload != NULL && valid_response(payload, NULL, error) != 0) {
    json_object_put(payload);
    payload = NULL;
  }
cleanup:
  curl_slist_free_all(headers);
  curl_easy_cleanup(curl);
  http_response_free(&response);
  return payload;
}

static json_object *get_response(struct api_client *client, const char *id,
                                 struct api_error *error) {
  const size_t length = strlen(id) + 16U;
  char *path = malloc(length);
  if (path == NULL) { (void)set_error(error, "Out of memory."); return NULL; }
  (void)snprintf(path, length, "/v1/responses/%s", id);
  struct http_response response;
  const int sent = http_request(client, "GET", path, NULL, false, true, &response, error);
  free(path);
  if (sent != 0) return NULL;
  json_object *payload = require_json_status(&response, 200L, error);
  http_response_free(&response);
  if (payload != NULL && valid_response(payload, id, error) != 0) {
    json_object_put(payload);
    return NULL;
  }
  return payload;
}

static json_object *wait_response(struct api_client *client, const char *id,
                                  double interval, double timeout,
                                  struct api_error *error) {
  const double deadline = monotonic_seconds() + timeout;
  char previous[32] = "";
  while (true) {
    json_object *value = get_response(client, id, error);
    if (value == NULL) return NULL;
    const char *status = json_string_field(value, "status");
    if (strcmp(status, previous) != 0) {
      fprintf(stderr, "Status: %s\n", status);
      (void)snprintf(previous, sizeof(previous), "%s", status);
    }
    if (strcmp(status, "completed") == 0 || strcmp(status, "failed") == 0) return value;
    json_object_put(value);
    const double remaining = deadline - monotonic_seconds();
    if (remaining <= 0.0) {
      (void)set_error(error, "Local polling timed out. The response continues and may be polled with ID %s.", id);
      return NULL;
    }
    sleep_seconds(interval < remaining ? interval : remaining);
  }
}

static const char *response_text(json_object *response, struct api_error *error) {
  const char *status = json_string_field(response, "status");
  if (strcmp(status, "failed") == 0) {
    json_object *failure = NULL;
    const char *code = "JV-AGENT";
    const char *message = "The structured response failed.";
    if (json_object_object_get_ex(response, "error", &failure) && json_object_is_type(failure, json_type_object)) {
      const char *candidate = json_string_field(failure, "code");
      if (candidate != NULL) code = candidate;
      candidate = json_string_field(failure, "message");
      if (candidate != NULL) message = candidate;
    }
    (void)set_error(error, "%s: %s", code, message);
    return NULL;
  }
  json_object *output = NULL;
  json_object_object_get_ex(response, "output", &output);
  json_object *item = json_object_array_get_idx(output, 0U);
  json_object *content = NULL;
  if (item == NULL || strcmp(json_string_field(item, "type") == NULL ? "" : json_string_field(item, "type"), "message") != 0 ||
      strcmp(json_string_field(item, "role") == NULL ? "" : json_string_field(item, "role"), "assistant") != 0 ||
      strcmp(json_string_field(item, "status") == NULL ? "" : json_string_field(item, "status"), "completed") != 0 ||
      !json_object_object_get_ex(item, "content", &content) || !json_object_is_type(content, json_type_array) || json_object_array_length(content) != 1U) {
    (void)set_error(error, "Expected one completed assistant text message.");
    return NULL;
  }
  json_object *part = json_object_array_get_idx(content, 0U);
  if (part == NULL || strcmp(json_string_field(part, "type") == NULL ? "" : json_string_field(part, "type"), "output_text") != 0) {
    (void)set_error(error, "Expected one completed assistant text message.");
    return NULL;
  }
  const char *text = json_string_field(part, "text");
  if (text == NULL) (void)set_error(error, "The assistant text is invalid.");
  return text;
}

static int get_tool_call(json_object *response, const char **call_id,
                         struct api_error *error) {
  json_object *output = NULL;
  json_object_object_get_ex(response, "output", &output);
  json_object *item = json_object_array_get_idx(output, 0U);
  const char *type = json_string_field(item, "type");
  const char *status = json_string_field(item, "status");
  const char *name = json_string_field(item, "name");
  const char *arguments_text = json_string_field(item, "arguments");
  *call_id = json_string_field(item, "call_id");
  if (type == NULL || strcmp(type, "function_call") != 0 || status == NULL ||
      strcmp(status, "completed") != 0 || name == NULL ||
      strcmp(name, "get_client_platform") != 0 || *call_id == NULL ||
      **call_id == '\0' || arguments_text == NULL)
    return set_error(error, "Expected the allowlisted get_client_platform call.");
  json_object *arguments = json_tokener_parse(arguments_text);
  const bool valid = arguments != NULL && json_object_is_type(arguments, json_type_object) &&
                     json_object_object_length(arguments) == 0;
  if (arguments != NULL) json_object_put(arguments);
  return valid ? 0 : set_error(error, "The platform tool accepts no arguments.");
}

static json_object *new_base_request(const char *question, bool tool_demo,
                                     struct api_error *error) {
  json_object *request = json_object_new_object();
  json_object *input = json_object_new_array();
  json_object *message = json_object_new_object();
  if (request == NULL || input == NULL || message == NULL ||
      add_json_value(request, "model", json_object_new_string("jv-ai"), error) != 0 ||
      add_json_value(request, "background", json_object_new_boolean(true), error) != 0 ||
      add_json_value(message, "role", json_object_new_string("user"), error) != 0 ||
      add_json_value(message, "content", json_object_new_string(question), error) != 0 ||
      json_object_array_add(input, message) != 0 ||
      add_json_value(request, "input", input, error) != 0) {
    if (request != NULL) json_object_put(request);
    return NULL;
  }
  json_object *tools = json_object_new_array();
  if (tools == NULL) {
    json_object_put(request);
    (void)set_error(error, "Could not prepare JSON request data.");
    return NULL;
  }
  if (tool_demo) {
    json_object *tool = json_object_new_object();
    json_object *parameters = json_object_new_object();
    if (tool == NULL || parameters == NULL ||
        add_json_value(parameters, "type", json_object_new_string("object"), error) != 0 ||
        add_json_value(parameters, "properties", json_object_new_object(), error) != 0 ||
        add_json_value(parameters, "required", json_object_new_array(), error) != 0 ||
        add_json_value(parameters, "additionalProperties", json_object_new_boolean(false), error) != 0 ||
        add_json_value(tool, "type", json_object_new_string("function"), error) != 0 ||
        add_json_value(tool, "name", json_object_new_string("get_client_platform"), error) != 0 ||
        add_json_value(tool, "description", json_object_new_string("Return the operating-system family of this client."), error) != 0 ||
        add_json_value(tool, "strict", json_object_new_boolean(true), error) != 0 ||
        add_json_value(tool, "parameters", parameters, error) != 0 ||
        json_object_array_add(tools, tool) != 0 ||
        add_json_value(request, "instructions", json_object_new_string("Call get_client_platform once. After its result arrives, answer briefly without requesting another tool."), error) != 0) {
      json_object_put(request);
      return NULL;
    }
  }
  if (tools == NULL || add_json_value(request, "tools", tools, error) != 0 ||
      add_json_value(request, "tool_choice", json_object_new_string(tool_demo ? "required" : "none"), error) != 0 ||
      add_json_value(request, "parallel_tool_calls", json_object_new_boolean(false), error) != 0 ||
      add_json_value(request, "store", json_object_new_boolean(true), error) != 0 ||
      add_json_value(request, "stream", json_object_new_boolean(false), error) != 0) {
    json_object_put(request);
    return NULL;
  }
  return request;
}

static json_object *new_continuation(const char *previous_id, const char *call_id,
                                     struct api_error *error) {
#ifdef _WIN32
  const char *platform = "windows";
#elif defined(__APPLE__)
  const char *platform = "macos";
#elif defined(__linux__)
  const char *platform = "linux";
#else
  const char *platform = "unknown";
#endif
  json_object *request = json_object_new_object();
  json_object *input = json_object_new_array();
  json_object *result = json_object_new_object();
  if (request == NULL || input == NULL || result == NULL ||
      add_json_value(request, "model", json_object_new_string("jv-ai"), error) != 0 ||
      add_json_value(request, "background", json_object_new_boolean(true), error) != 0 ||
      add_json_value(request, "previous_response_id", json_object_new_string(previous_id), error) != 0 ||
      add_json_value(request, "instructions", json_object_new_string("Use the trusted tool result and answer the original request."), error) != 0 ||
      add_json_value(result, "type", json_object_new_string("function_call_output"), error) != 0 ||
      add_json_value(result, "call_id", json_object_new_string(call_id), error) != 0 ||
      add_json_value(result, "output", json_object_new_string(platform), error) != 0 ||
      json_object_array_add(input, result) != 0 ||
      add_json_value(request, "input", input, error) != 0 ||
      add_json_value(request, "tools", json_object_new_array(), error) != 0 ||
      add_json_value(request, "tool_choice", json_object_new_string("none"), error) != 0 ||
      add_json_value(request, "parallel_tool_calls", json_object_new_boolean(false), error) != 0 ||
      add_json_value(request, "store", json_object_new_boolean(true), error) != 0 ||
      add_json_value(request, "stream", json_object_new_boolean(false), error) != 0) {
    if (request != NULL) json_object_put(request);
    return NULL;
  }
  return request;
}

int main(int argc, char **argv) {
  struct api_error error = {0};
  struct responses_options options;
  if (response_options(argc, argv, &options, &error) != 0) { fprintf(stderr, "Error: %s\n", error.message); return 1; }
  if (options.help) { responses_usage(argv[0]); return 0; }
  if (curl_global_init(CURL_GLOBAL_DEFAULT) != CURLE_OK) { fputs("Error: Could not initialize libcurl.\n", stderr); return 1; }
  int result = 1;
  struct api_client client;
  memset(&client, 0, sizeof(client));
  char *password = NULL;
  json_object *request = NULL;
  json_object *created = NULL;
  json_object *terminal = NULL;
  if (client_initialize(&client, options.base_url, &error) != 0) goto cleanup;
  const char *configured = getenv("JV_API_PASSWORD");
  password = configured == NULL ? read_password(options.username, &error) : duplicate_string(configured);
  if (password == NULL || client_login(&client, options.username, password, &error) != 0) goto cleanup;
  clear_secret(password); free(password); password = NULL;
  request = new_base_request(options.question, options.tool_demo, &error);
  if (request == NULL) goto cleanup;
  created = create_response(&client, request, &error);
  if (created == NULL) goto cleanup;
  const char *id = json_string_field(created, "id");
  fprintf(stderr, "Created structured response %s.\n", id);
  terminal = wait_response(&client, id, options.poll_interval, options.wait_timeout, &error);
  if (terminal == NULL) goto cleanup;
  if (options.tool_demo) {
    const char *call_id = NULL;
    if (get_tool_call(terminal, &call_id, &error) != 0) goto cleanup;
    char *call_id_copy = duplicate_string(call_id);
    char *previous_id = duplicate_string(id);
    if (call_id_copy == NULL || previous_id == NULL) {
      free(call_id_copy); free(previous_id);
      (void)set_error(&error, "Out of memory.");
      goto cleanup;
    }
    fprintf(stderr, "Executing allowlisted local tool get_client_platform; JV Server does not execute it.\n");
    json_object_put(terminal); terminal = NULL;
    json_object_put(created); created = NULL;
    json_object_put(request); request = new_continuation(previous_id, call_id_copy, &error);
    free(call_id_copy); free(previous_id);
    if (request == NULL) goto cleanup;
    created = create_response(&client, request, &error);
    if (created == NULL) goto cleanup;
    id = json_string_field(created, "id");
    fprintf(stderr, "Created continuation response %s.\n", id);
    terminal = wait_response(&client, id, options.poll_interval, options.wait_timeout, &error);
    if (terminal == NULL) goto cleanup;
  }
  const char *answer = response_text(terminal, &error);
  if (answer == NULL) goto cleanup;
  puts(options.print_json ? json_object_to_json_string_ext(terminal, JSON_C_TO_STRING_PRETTY) : answer);
  result = 0;
cleanup:
  clear_secret(password); free(password);
  if (terminal != NULL) json_object_put(terminal);
  if (created != NULL) json_object_put(created);
  if (request != NULL) json_object_put(request);
  if (client.access_token != NULL) { struct api_error logout_error = {0}; if (client_logout(&client, &logout_error) != 0) fprintf(stderr, "Warning: %s\n", logout_error.message); }
  client_destroy(&client);
  curl_global_cleanup();
  if (result != 0) fprintf(stderr, "Error: %s\n", *error.message == '\0' ? "Unexpected failure." : error.message);
  return result;
}
