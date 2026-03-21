/**
 * Generate a prefixed unique ID.
 * @param {string} prefix - ID prefix (e.g. "u", "p")
 * @returns {string}
 */
let counter = 0;
function genId(prefix) {
  return `${prefix}-${++counter}`;
}

/**
 * Deep-clone a plain object.
 * @param {Object} obj
 * @returns {Object}
 */
function deepClone(obj) {
  return JSON.parse(JSON.stringify(obj));
}

/**
 * Validate that required fields are present and non-empty.
 * @param {Object} obj - Object to check
 * @param {string[]} fields - Required field names
 * @returns {{ valid: boolean, missing: string[] }}
 */
function validateRequired(obj, fields) {
  const missing = fields.filter((f) => !obj[f]);
  return { valid: missing.length === 0, missing };
}

/**
 * Create a user object.
 * @param {string} name
 * @param {string} email
 * @param {string} [role="viewer"]
 * @returns {Object}
 */
function createUser(name, email, role = "viewer") {
  const user = { id: genId("u"), name, email, role };
  const check = validateRequired(user, ["name", "email"]);
  if (!check.valid) {
    throw new Error(`missing fields: ${check.missing.join(", ")}`);
  }
  return user;
}

/**
 * Check if a user is an admin.
 * @param {Object} user
 * @returns {boolean}
 */
function isAdmin(user) {
  return user.role === "admin";
}

module.exports = { genId, deepClone, validateRequired, createUser, isAdmin };
